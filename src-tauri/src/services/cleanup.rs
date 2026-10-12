//! 数据清理与彻底清空（T14）。
//!
//! 依据：
//! - `docs/specs/SECURITY_PRIVACY.md` 第 6 节：清理统计明细默认保留去重身份；彻底清空则删身份/
//!   游标并提示可重新导入；清除账户条目同时按引用清理凭据，历史快照是否保留由明确选项决定；
//!   文件删除不保证底层介质不可恢复，文案不使用“永久安全擦除”。
//! - `docs/specs/DATA_CONTRACTS.md` 第 4 节（墓碑）与第 9 节（表清单）。
//! - `docs/ROADMAP_AND_ACCEPTANCE.md` C15/T14“删除可重建”。
//!
//! 两个域互不影响：统计清理只动 `usage_events` / `usage_observations` /
//! `event_identity_tombstones` / `ingestion_cursors` / `parse_runs` / `sessions` / `actors`；
//! 账户清理只动 `account_snapshots` / `balances` / `quota_windows`（以及删除账户时的凭据引用）。
//!
//! 多表写操作在单事务内完成；删除顺序照顾外键（先删被引用方）。

use crate::ingestion;
use crate::services::accounts;
use crate::storage::{Database, StorageError};
use crate::vault::Vault;
use serde::Serialize;

/// 明细清理后墓碑保留、旧文件不会再次计入（数据契约第 4 节）。
pub const CLEAR_KEEPS_IDENTITY_KEY: &str = "cleanup.clear.tombstone_kept";
/// 彻底清空后身份/游标一并删除，随后同文件重扫可能重新导入。
pub const PURGE_REIMPORT_HINT_KEY: &str = "cleanup.purge.reimport_possible";
/// 删除账户：历史快照保留（默认选项）。
pub const ACCOUNT_HISTORY_KEPT_KEY: &str = "cleanup.account.history_kept";
/// 删除账户：历史快照一并删除（显式选项）。
pub const ACCOUNT_HISTORY_DELETED_KEY: &str = "cleanup.account.history_deleted";
/// 删除账户：按引用清理凭据（不等于平台撤销 Key）。
pub const ACCOUNT_CREDENTIALS_CLEARED_KEY: &str = "cleanup.account.credentials_cleared";
/// 文件删除不保证底层介质不可恢复，不使用“永久安全擦除”表述。
pub const MEDIA_NOT_WIPED_KEY: &str = "cleanup.media.not_guaranteed_unrecoverable";

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Ingestion(#[from] ingestion::IngestionError),
    #[error(transparent)]
    Accounts(#[from] accounts::AccountsError),
}

pub type Result<T> = std::result::Result<T, CleanupError>;

/// 明细清理墓碑的 `reason`：绑定 source，便于 [`purge_statistics`] 精确回收本源的墓碑
/// （`event_identity_tombstones` 表结构没有 `source_id` 列）。
fn clear_reason(source_id: &str) -> String {
    format!("source_clear:{source_id}")
}

// ---------------------------------------------------------------------------
// 统计清理
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearDetailResult {
    pub source_id: String,
    pub events_removed: u64,
    /// 新建的身份墓碑数（= 保留的去重身份）。
    pub identities_retired: u64,
    pub note_key: &'static str,
}

/// 清理某个源的统计明细，但**保留去重身份**（写墓碑）。
///
/// 旧文件再次扫描时墓碑会拦截同一 `event_id`，不会重新计入。要允许重新导入用
/// [`purge_statistics`]。
pub fn clear_statistics_detail(db: &mut Database, source_id: &str) -> Result<ClearDetailResult> {
    let events_removed: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM usage_events WHERE source_id = ?1",
        [source_id],
        |r| r.get(0),
    )?;
    let identities_retired =
        ingestion::retire_events_with_tombstones(db, source_id, &clear_reason(source_id))?;
    Ok(ClearDetailResult {
        source_id: source_id.to_string(),
        events_removed: events_removed.max(0) as u64,
        identities_retired,
        note_key: CLEAR_KEEPS_IDENTITY_KEY,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeResult {
    pub source_id: String,
    pub events_removed: u64,
    pub observations_removed: u64,
    pub tombstones_removed: u64,
    pub cursors_removed: u64,
    pub parse_runs_removed: u64,
    pub sessions_removed: u64,
    pub actors_removed: u64,
    /// 前端据此提示“随后重扫同一文件可能重新导入”。
    pub reimport_hint_key: &'static str,
}

/// 彻底清空某个源的统计：事件 + 观察 + 墓碑 + 游标 + `parse_runs` + 会话索引（含 actors），
/// 单事务完成。随后同文件重扫会重新导入（返回 `reimport_hint_key` 供 UI 提示）。
///
/// 墓碑回收范围：本模块 [`clear_statistics_detail`] 为本源写下的墓碑（`reason` 含 source_id）。
/// 扫描器因文件换代写下的 `file_regeneration` 墓碑不带 source 归属，本函数不触碰，
/// 以免影响其它源——这是已知边界。
pub fn purge_statistics(db: &mut Database, source_id: &str) -> Result<PurgeResult> {
    let reason = clear_reason(source_id);
    let tx = db.transaction()?;

    let count = |sql: &str, param: &str| -> std::result::Result<i64, rusqlite::Error> {
        tx.query_row(sql, [param], |r| r.get(0))
    };
    let observations_removed = count(
        "SELECT COUNT(*) FROM usage_observations WHERE source_id = ?1",
        source_id,
    )?;
    let events_removed = count(
        "SELECT COUNT(*) FROM usage_events WHERE source_id = ?1",
        source_id,
    )?;
    let tombstones_removed = count(
        "SELECT COUNT(*) FROM event_identity_tombstones WHERE reason = ?1",
        &reason,
    )?;
    let cursors_removed = count(
        "SELECT COUNT(*) FROM ingestion_cursors WHERE source_id = ?1",
        source_id,
    )?;
    let parse_runs_removed = count(
        "SELECT COUNT(*) FROM parse_runs WHERE source_id = ?1",
        source_id,
    )?;
    let sessions_removed = count(
        "SELECT COUNT(*) FROM sessions WHERE source_id = ?1",
        source_id,
    )?;
    let actors_removed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM actors WHERE session_key IN
            (SELECT session_key FROM sessions WHERE source_id = ?1)",
        [source_id],
        |r| r.get(0),
    )?;

    // 删除顺序：先删被引用方（observations → events；sessions 会级联 actors），
    // 再删 parse_runs（被 usage_events.parse_run_id 引用，事件已删才安全）。
    tx.execute(
        "DELETE FROM usage_observations WHERE source_id = ?1",
        [source_id],
    )?;
    tx.execute("DELETE FROM usage_events WHERE source_id = ?1", [source_id])?;
    tx.execute(
        "DELETE FROM event_identity_tombstones WHERE reason = ?1",
        [&reason],
    )?;
    tx.execute(
        "DELETE FROM ingestion_cursors WHERE source_id = ?1",
        [source_id],
    )?;
    tx.execute("DELETE FROM parse_runs WHERE source_id = ?1", [source_id])?;
    tx.execute("DELETE FROM sessions WHERE source_id = ?1", [source_id])?;
    tx.commit()?;

    Ok(PurgeResult {
        source_id: source_id.to_string(),
        events_removed: events_removed.max(0) as u64,
        observations_removed: observations_removed.max(0) as u64,
        tombstones_removed: tombstones_removed.max(0) as u64,
        cursors_removed: cursors_removed.max(0) as u64,
        parse_runs_removed: parse_runs_removed.max(0) as u64,
        sessions_removed: sessions_removed.max(0) as u64,
        actors_removed: actors_removed.max(0) as u64,
        reimport_hint_key: PURGE_REIMPORT_HINT_KEY,
    })
}

// ---------------------------------------------------------------------------
// 账户清理
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotClearResult {
    pub account_id: String,
    pub snapshots_removed: u64,
    pub balances_removed: u64,
    pub quota_windows_removed: u64,
    pub usage_metrics_removed: u64,
}

/// 删除某账户的全部历史快照（含余额、额度窗口与用量指标），单事务。
///
/// **不触碰**账户行、凭据引用与任何用量账本表；统计清理与账户清理互不影响。
pub fn clear_account_snapshots(db: &mut Database, account_id: &str) -> Result<SnapshotClearResult> {
    let tx = db.transaction()?;
    let snapshots_removed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM account_snapshots WHERE account_id = ?1",
        [account_id],
        |r| r.get(0),
    )?;
    let balances_removed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM balances WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
        |r| r.get(0),
    )?;
    let quota_windows_removed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM quota_windows WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
        |r| r.get(0),
    )?;
    let usage_metrics_removed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM usage_metrics WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "DELETE FROM balances WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
    )?;
    tx.execute(
        "DELETE FROM quota_windows WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
    )?;
    tx.execute(
        "DELETE FROM usage_metrics WHERE snapshot_id IN
            (SELECT snapshot_id FROM account_snapshots WHERE account_id = ?1)",
        [account_id],
    )?;
    tx.execute(
        "DELETE FROM account_snapshots WHERE account_id = ?1",
        [account_id],
    )?;
    tx.commit()?;

    Ok(SnapshotClearResult {
        account_id: account_id.to_string(),
        snapshots_removed: snapshots_removed.max(0) as u64,
        balances_removed: balances_removed.max(0) as u64,
        quota_windows_removed: quota_windows_removed.max(0) as u64,
        usage_metrics_removed: usage_metrics_removed.max(0) as u64,
    })
}

/// 凭据清理摘要的可序列化视图（`accounts::CredentialCleanup` 不含 serde 派生）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialCleanupSummary {
    pub refs_removed: u64,
    pub vault_entries_removed: u64,
    pub shared_refs_kept: u64,
}

impl From<accounts::CredentialCleanup> for CredentialCleanupSummary {
    fn from(c: accounts::CredentialCleanup) -> Self {
        Self {
            refs_removed: c.refs_removed as u64,
            vault_entries_removed: c.vault_entries_removed as u64,
            shared_refs_kept: c.shared_refs_kept as u64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeleteResult {
    pub account_id: String,
    /// 软删除账户 + 按引用清理凭据（复用账户域逻辑）。
    pub credentials: CredentialCleanupSummary,
    pub snapshots_removed: u64,
    pub history_kept: bool,
    pub note_keys: Vec<&'static str>,
}

/// 彻底删除账户。
///
/// - 账户行**软删除**（`deleted_at` 置位，行与时间信息保留），并按引用清理凭据
///   （复用 [`accounts::account_delete`]；凭据库不在 SQLite 事务内，属尽力清理）。
/// - `keep_history`：`true` 保留历史快照（默认策略）；`false` 连同快照/余额/额度窗口一并删除。
/// - 只按引用清理本地凭据，**不等于**平台撤销 Key；文件删除不保证介质不可恢复。
pub fn delete_account_fully(
    db: &mut Database,
    vault: &dyn Vault,
    account_id: &str,
    keep_history: bool,
) -> Result<AccountDeleteResult> {
    let cleanup = accounts::account_delete(db, vault, account_id)?;
    let snapshots_removed = if keep_history {
        0
    } else {
        clear_account_snapshots(db, account_id)?.snapshots_removed
    };
    let mut note_keys = vec![
        if keep_history {
            ACCOUNT_HISTORY_KEPT_KEY
        } else {
            ACCOUNT_HISTORY_DELETED_KEY
        },
        ACCOUNT_CREDENTIALS_CLEARED_KEY,
        MEDIA_NOT_WIPED_KEY,
    ];
    note_keys.dedup();
    Ok(AccountDeleteResult {
        account_id: account_id.to_string(),
        credentials: cleanup.into(),
        snapshots_removed,
        history_kept: keep_history,
        note_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::UsageEvent;
    use crate::ingestion::{commit_batch, start_parse_run, CursorUpdate, EventInput, ImportBatch};

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO sources (source_id, agent, stream_domain, root_path, parser_version)
                 VALUES ('src1', 'kimi_code', 'local_log', '/anon', 'kimi-wire-1.5')",
                [],
            )
            .unwrap();
        db
    }

    fn sample_event(id: &str) -> UsageEvent {
        UsageEvent {
            schema_version: 1,
            event_id: id.into(),
            agent: crate::domain::AgentKind::KimiCode,
            source_id: "src1".into(),
            stream_domain: crate::domain::StreamDomain::LocalLog,
            session_id: Some("s1".into()),
            parent_session_id: None,
            actor_id: Some("main".into()),
            turn_id: None,
            project_ref: None,
            occurred_at: Some("2026-10-11T06:00:00Z".into()),
            observed_at: "2026-10-11T06:00:01Z".into(),
            model_raw: Some("kimi-k2".into()),
            model_normalized: None,
            granularity: crate::domain::Granularity::Request,
            request_count: Some(1),
            tokens: crate::domain::TokenSet {
                input_fresh: Some(100),
                input_cache_read: Some(40),
                input_cache_write: Some(0),
                input_total: Some(140),
                output_total: Some(30),
                output_reasoning: None,
                total: Some(170),
                quality: crate::domain::Quality::Reported,
            },
            cost_estimated: None,
            completeness: crate::domain::Completeness::Complete,
            account_link: None,
            evidence_refs: vec![crate::domain::EvidenceRef {
                source_id: "src1".into(),
                file_identity: "fileA".into(),
                generation: 0,
                locator: "1".into(),
                parser_version: "kimi-wire-1.5".into(),
            }],
        }
    }

    fn import_one(db: &mut Database, id: &str) {
        let run = start_parse_run(db, "src1", "kimi-wire-1.5").unwrap();
        let batch = ImportBatch {
            source_id: "src1".into(),
            events: vec![EventInput {
                event: sample_event(id),
                replace_existing: false,
            }],
            sessions: vec![],
            cursor: Some(CursorUpdate {
                file_identity: "fileA".into(),
                generation: 0,
                parser_version: "kimi-wire-1.5".into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: "1".into(),
                tail_checksum: None,
            }),
        };
        commit_batch(db, run, &batch).unwrap();
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn clear_keeps_identity_and_purge_allows_reimport() {
        let mut db = setup_db();
        import_one(&mut db, "e1");
        assert_eq!(count(&db, "usage_events"), 1);
        assert_eq!(count(&db, "ingestion_cursors"), 1);

        let cleared = clear_statistics_detail(&mut db, "src1").unwrap();
        assert_eq!(cleared.events_removed, 1);
        assert_eq!(cleared.identities_retired, 1);
        assert_eq!(count(&db, "usage_events"), 0);
        assert_eq!(count(&db, "event_identity_tombstones"), 1);
        // 游标仍在（明细清理不重置扫描进度）
        assert_eq!(count(&db, "ingestion_cursors"), 1);

        // 清理后重扫同一事件：墓碑拦截
        let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let stats = commit_batch(
            &mut db,
            run,
            &ImportBatch {
                source_id: "src1".into(),
                events: vec![EventInput {
                    event: sample_event("e1"),
                    replace_existing: false,
                }],
                sessions: vec![],
                cursor: None,
            },
        )
        .unwrap();
        assert_eq!(stats.tombstoned, 1);
        assert_eq!(count(&db, "usage_events"), 0);

        // 彻底清空：墓碑与游标一并删除
        let purged = purge_statistics(&mut db, "src1").unwrap();
        assert_eq!(purged.tombstones_removed, 1);
        assert_eq!(purged.cursors_removed, 1);
        assert_eq!(purged.parse_runs_removed, 2);
        assert_eq!(purged.reimport_hint_key, PURGE_REIMPORT_HINT_KEY);
        assert_eq!(count(&db, "event_identity_tombstones"), 0);
        assert_eq!(count(&db, "ingestion_cursors"), 0);

        // 清空后重扫可重新导入
        import_one(&mut db, "e1");
        assert_eq!(count(&db, "usage_events"), 1);
    }

    #[test]
    fn purge_removes_sessions_and_observations() {
        let mut db = setup_db();
        import_one(&mut db, "e1");
        // commit_batch 未建会话，手动插入会话与 actor 验证会话索引清理
        db.conn()
            .execute(
                "INSERT INTO sessions (session_key, source_id, agent, session_id)
                 VALUES ('src1:s1', 'src1', 'kimi_code', 's1')",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO actors (session_key, actor_id) VALUES ('src1:s1', 'main')",
                [],
            )
            .unwrap();
        assert_eq!(count(&db, "usage_observations"), 1);

        let purged = purge_statistics(&mut db, "src1").unwrap();
        assert_eq!(purged.events_removed, 1);
        assert_eq!(purged.observations_removed, 1);
        assert_eq!(purged.sessions_removed, 1);
        assert_eq!(purged.actors_removed, 1);
        assert_eq!(count(&db, "sessions"), 0);
        assert_eq!(count(&db, "actors"), 0);
    }
}

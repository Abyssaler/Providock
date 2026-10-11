//! 采集入库：一次提交事务 = 事件 upsert + 观察证据 + 会话更新 + 游标更新
//! （ARCHITECTURE 第 5 节）。任一步失败全部回滚；未写完的尾行不推进游标
//! （由适配器保证游标只指向已完整解析的位置）。
//!
//! 身份与去重（DATA_CONTRACTS 第 4 节）：
//! - event_id 由适配器按原生 ID / 会话+位置规则生成，本层不发明身份。
//! - 已存在的 event_id 默认视为同一逻辑事件的重复观察，只补观察证据；
//!   消息的部分 usage 更新用 replace_existing 替换旧版本，不逐次累加。
//! - 墓碑中的 event_id 直接跳过，防止清理后旧文件再次计入。

use crate::domain::UsageEvent;
use crate::storage::{token_to_sql, Database, StorageError};

#[derive(Debug, thiserror::Error)]
pub enum IngestionError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, IngestionError>;

pub struct EventInput {
    pub event: UsageEvent,
    /// true = 同一逻辑事件的新版本（如流式补全），替换旧计量而非忽略
    pub replace_existing: bool,
}

pub struct SessionUpsert {
    pub session_id: String,
    pub parent_session_key: Option<String>,
    pub title: Option<String>,
    pub project_ref: Option<String>,
    pub last_seen_at: Option<String>,
    pub archived: bool,
    pub has_subagents: bool,
}

pub struct CursorUpdate {
    pub file_identity: String,
    pub generation: u64,
    pub parser_version: String,
    /// byte_offset / row_id / composite
    pub cursor_kind: String,
    pub cursor_value: String,
    pub tail_checksum: Option<String>,
}

#[derive(Default)]
pub struct ImportBatch {
    pub source_id: String,
    pub events: Vec<EventInput>,
    pub sessions: Vec<SessionUpsert>,
    pub cursor: Option<CursorUpdate>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub inserted: u64,
    pub replaced: u64,
    pub already_present: u64,
    pub tombstoned: u64,
}

pub fn start_parse_run(db: &Database, source_id: &str, parser_version: &str) -> Result<i64> {
    db.conn().execute(
        "INSERT INTO parse_runs (source_id, parser_version) VALUES (?1, ?2)",
        rusqlite::params![source_id, parser_version],
    )?;
    Ok(db.conn().last_insert_rowid())
}

pub fn finish_parse_run(db: &Database, run_id: i64, ok: bool, error: Option<&str>) -> Result<()> {
    db.conn().execute(
        "UPDATE parse_runs SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
            status = ?2, error = ?3 WHERE run_id = ?1",
        rusqlite::params![run_id, if ok { "committed" } else { "failed" }, error],
    )?;
    Ok(())
}

/// 在一个事务里提交一批事件、会话更新与游标。
pub fn commit_batch(db: &mut Database, run_id: i64, batch: &ImportBatch) -> Result<CommitStats> {
    let tx = db.transaction()?;
    let mut stats = CommitStats::default();

    {
        let mut tombstone_stmt =
            tx.prepare("SELECT 1 FROM event_identity_tombstones WHERE event_id = ?1")?;
        let mut insert_event = tx.prepare(
            "INSERT OR IGNORE INTO usage_events (
                event_id, schema_version, parse_run_id, source_id, agent, stream_domain,
                session_id, parent_session_id, actor_id, turn_id, project_ref,
                occurred_at, observed_at, model_raw, model_normalized, granularity,
                request_count,
                input_fresh, input_cache_read, input_cache_write, input_total,
                output_total, output_reasoning, total, tokens_quality, completeness,
                cost_reported_amount, cost_reported_currency, cost_reported_completeness,
                cost_estimated_amount, cost_estimated_currency, cost_estimated_price_version,
                account_link, evidence_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34
            )",
        )?;
        let mut update_event = tx.prepare(
            "UPDATE usage_events SET
                parse_run_id = ?3, occurred_at = ?12, observed_at = ?13,
                model_raw = ?14, model_normalized = ?15, granularity = ?16,
                request_count = ?17,
                input_fresh = ?18, input_cache_read = ?19, input_cache_write = ?20,
                input_total = ?21, output_total = ?22, output_reasoning = ?23, total = ?24,
                tokens_quality = ?25, completeness = ?26,
                cost_reported_amount = ?27, cost_reported_currency = ?28,
                cost_reported_completeness = ?29,
                cost_estimated_amount = ?30, cost_estimated_currency = ?31,
                cost_estimated_price_version = ?32,
                account_link = ?33, evidence_json = ?34
            WHERE event_id = ?1",
        )?;
        let mut insert_observation = tx.prepare(
            "INSERT OR IGNORE INTO usage_observations
                (event_id, source_id, file_identity, generation, locator, parser_version)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        for input in &batch.events {
            let event = &input.event;
            if tombstone_stmt.exists(rusqlite::params![&event.event_id])? {
                stats.tombstoned += 1;
                continue;
            }

            let params = event_params(event, run_id)?;
            let changed = insert_event.execute(rusqlite::params_from_iter(params.iter()))?;
            if changed == 1 {
                stats.inserted += 1;
            } else if input.replace_existing {
                // 同一逻辑事件的新版本：替换计量，不产生第二条事件
                update_event.execute(rusqlite::params_from_iter(params.iter()))?;
                stats.replaced += 1;
            } else {
                stats.already_present += 1;
            }

            for ev in &event.evidence_refs {
                insert_observation.execute(rusqlite::params![
                    &event.event_id,
                    ev.source_id,
                    ev.file_identity,
                    token_to_sql(ev.generation)?,
                    ev.locator,
                    ev.parser_version,
                ])?;
            }
        }
    }

    {
        let mut upsert_session = tx.prepare(
            "INSERT INTO sessions (session_key, source_id, agent, session_id, parent_session_key,
                                   title, project_ref, last_seen_at, archived, has_subagents)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(session_key) DO UPDATE SET
                last_seen_at = MAX(COALESCE(sessions.last_seen_at, ''), COALESCE(excluded.last_seen_at, '')),
                archived = excluded.archived,
                has_subagents = excluded.has_subagents,
                parent_session_key = COALESCE(excluded.parent_session_key, sessions.parent_session_key),
                title = COALESCE(excluded.title, sessions.title),
                project_ref = COALESCE(excluded.project_ref, sessions.project_ref)",
        )?;
        // agent 取自本批事件；会话可能早于事件建立索引
        for s in &batch.sessions {
            let session_key = format!("{}:{}", batch.source_id, s.session_id);
            let agent = batch
                .events
                .iter()
                .find_map(|e| {
                    if e.event.session_id.as_deref() == Some(s.session_id.as_str()) {
                        Some(e.event.agent.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("unknown");
            upsert_session.execute(rusqlite::params![
                session_key,
                batch.source_id,
                agent,
                s.session_id,
                s.parent_session_key,
                s.title,
                s.project_ref,
                s.last_seen_at,
                s.archived as i64,
                s.has_subagents as i64,
            ])?;
        }
    }

    if let Some(c) = &batch.cursor {
        tx.execute(
            "INSERT INTO ingestion_cursors
                (source_id, file_identity, generation, parser_version, cursor_kind, cursor_value, tail_checksum)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source_id, file_identity, generation) DO UPDATE SET
                parser_version = excluded.parser_version,
                cursor_kind = excluded.cursor_kind,
                cursor_value = excluded.cursor_value,
                tail_checksum = excluded.tail_checksum,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            rusqlite::params![
                batch.source_id,
                c.file_identity,
                token_to_sql(c.generation)?,
                c.parser_version,
                c.cursor_kind,
                c.cursor_value,
                c.tail_checksum,
            ],
        )?;
    }

    tx.execute(
        "UPDATE parse_runs SET events_written = events_written + ?2 WHERE run_id = ?1",
        rusqlite::params![run_id, stats.inserted as i64],
    )?;

    tx.commit()?;
    Ok(stats)
}

fn event_params(event: &UsageEvent, run_id: i64) -> Result<Vec<rusqlite::types::Value>> {
    use rusqlite::types::Value;
    let t = &event.tokens;
    let opt = |v: Option<u64>| -> Result<Value> {
        Ok(match v {
            Some(v) => Value::Integer(token_to_sql(v)?),
            None => Value::Null,
        })
    };
    let text = |v: &Option<String>| Value::from(v.clone());
    let (cr_a, cr_c, cr_comp) = match &event.cost_reported {
        Some(c) => (
            Value::Text(c.money.amount.clone()),
            Value::Text(c.money.currency.clone()),
            Value::Text(c.completeness.as_str().to_string()),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    let (ce_a, ce_c, ce_pv) = match &event.cost_estimated {
        Some(c) => (
            Value::Text(c.money.amount.clone()),
            Value::Text(c.money.currency.clone()),
            Value::from(c.price_version.clone()),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    let account_link = match &event.account_link {
        Some(l) => Value::Text(serde_json::to_string(l).map_err(|e| {
            StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?),
        None => Value::Null,
    };
    let evidence = serde_json::to_string(&event.evidence_refs)
        .map_err(|e| StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;

    Ok(vec![
        Value::Text(event.event_id.clone()),
        Value::Integer(i64::from(event.schema_version)),
        Value::Integer(run_id),
        Value::Text(event.source_id.clone()),
        Value::Text(event.agent.as_str().into()),
        Value::Text(event.stream_domain.as_str().into()),
        text(&event.session_id),
        text(&event.parent_session_id),
        text(&event.actor_id),
        text(&event.turn_id),
        text(&event.project_ref),
        text(&event.occurred_at),
        Value::Text(event.observed_at.clone()),
        text(&event.model_raw),
        text(&event.model_normalized),
        Value::Text(event.granularity.as_str().into()),
        opt(event.request_count)?,
        opt(t.input_fresh)?,
        opt(t.input_cache_read)?,
        opt(t.input_cache_write)?,
        opt(t.input_total)?,
        opt(t.output_total)?,
        opt(t.output_reasoning)?,
        opt(t.total)?,
        Value::Text(t.quality.as_str().into()),
        Value::Text(event.completeness.as_str().into()),
        cr_a,
        cr_c,
        cr_comp,
        ce_a,
        ce_c,
        ce_pv,
        account_link,
        Value::Text(evidence),
    ])
}

/// 清理明细但保留身份墓碑（数据契约第 4 节）；
/// 彻底清空由独立的 purge 流程连同墓碑与游标一起删除。
pub fn retire_events_with_tombstones(
    db: &mut Database,
    source_id: &str,
    reason: &str,
) -> Result<u64> {
    let tx = db.transaction()?;
    let n = tx.execute(
        "INSERT OR IGNORE INTO event_identity_tombstones (event_id, reason)
         SELECT event_id, ?2 FROM usage_events WHERE source_id = ?1",
        rusqlite::params![source_id, reason],
    )?;
    tx.execute(
        "DELETE FROM usage_events WHERE source_id = ?1",
        rusqlite::params![source_id],
    )?;
    tx.commit()?;
    Ok(n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    use crate::storage::migrations;

    fn setup() -> Database {
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

    pub(crate) fn sample_event(id: &str) -> UsageEvent {
        UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: id.into(),
            agent: AgentKind::KimiCode,
            source_id: "src1".into(),
            stream_domain: StreamDomain::LocalLog,
            session_id: Some("s1".into()),
            parent_session_id: None,
            actor_id: Some("main".into()),
            turn_id: None,
            project_ref: None,
            occurred_at: Some("2026-10-11T06:00:00Z".into()),
            observed_at: "2026-10-11T06:00:01Z".into(),
            model_raw: Some("kimi-k2".into()),
            model_normalized: None,
            granularity: Granularity::Request,
            request_count: Some(1),
            tokens: TokenSet {
                input_fresh: Some(100),
                input_cache_read: Some(40),
                input_cache_write: Some(0),
                input_total: Some(140),
                output_total: Some(30),
                output_reasoning: None,
                total: Some(170),
                quality: Quality::Reported,
            },
            cost_reported: None,
            cost_estimated: None,
            completeness: Completeness::Complete,
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: "src1".into(),
                file_identity: "fileA".into(),
                generation: 0,
                locator: "1024".into(),
                parser_version: "kimi-wire-1.5".into(),
            }],
        }
    }

    fn batch(events: Vec<EventInput>) -> ImportBatch {
        ImportBatch {
            source_id: "src1".into(),
            events,
            sessions: vec![],
            cursor: Some(CursorUpdate {
                file_identity: "fileA".into(),
                generation: 0,
                parser_version: "kimi-wire-1.5".into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: "1024".into(),
                tail_checksum: None,
            }),
        }
    }

    fn event_count(db: &Database) -> i64 {
        db.conn()
            .query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))
            .unwrap()
    }

    fn total_tokens(db: &Database) -> Option<i64> {
        db.conn()
            .query_row("SELECT SUM(total) FROM usage_events", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn rescan_is_idempotent() {
        let mut db = setup();
        let run1 = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let stats = commit_batch(
            &mut db,
            run1,
            &batch(vec![
                EventInput {
                    event: sample_event("e1"),
                    replace_existing: false,
                },
                EventInput {
                    event: sample_event("e2"),
                    replace_existing: false,
                },
            ]),
        )
        .unwrap();
        assert_eq!(stats.inserted, 2);
        assert_eq!(total_tokens(&db), Some(340));

        // 重扫同一批：不重复计数
        let run2 = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let stats2 = commit_batch(
            &mut db,
            run2,
            &batch(vec![
                EventInput {
                    event: sample_event("e1"),
                    replace_existing: false,
                },
                EventInput {
                    event: sample_event("e2"),
                    replace_existing: false,
                },
            ]),
        )
        .unwrap();
        assert_eq!(stats2.already_present, 2);
        assert_eq!(stats2.inserted, 0);
        assert_eq!(event_count(&db), 2);
        assert_eq!(total_tokens(&db), Some(340));
    }

    #[test]
    fn message_update_replaces_instead_of_accumulating() {
        let mut db = setup();
        let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        commit_batch(
            &mut db,
            run,
            &batch(vec![EventInput {
                event: sample_event("e1"),
                replace_existing: false,
            }]),
        )
        .unwrap();

        // 同一逻辑事件的新版本（例如流式补全后 output 增大）
        let mut updated = sample_event("e1");
        updated.tokens.output_total = Some(50);
        updated.tokens.total = Some(190);
        let run2 = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let stats = commit_batch(
            &mut db,
            run2,
            &batch(vec![EventInput {
                event: updated,
                replace_existing: true,
            }]),
        )
        .unwrap();
        assert_eq!(stats.replaced, 1);
        assert_eq!(event_count(&db), 1);
        assert_eq!(total_tokens(&db), Some(190));
    }

    #[test]
    fn tombstone_blocks_reimport() {
        let mut db = setup();
        let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        commit_batch(
            &mut db,
            run,
            &batch(vec![EventInput {
                event: sample_event("e1"),
                replace_existing: false,
            }]),
        )
        .unwrap();
        let retired = retire_events_with_tombstones(&mut db, "src1", "user_cleared").unwrap();
        assert_eq!(retired, 1);
        assert_eq!(event_count(&db), 0);

        // 清理后重扫旧文件：墓碑拦截，不重新计入
        let run2 = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let stats = commit_batch(
            &mut db,
            run2,
            &batch(vec![EventInput {
                event: sample_event("e1"),
                replace_existing: false,
            }]),
        )
        .unwrap();
        assert_eq!(stats.tombstoned, 1);
        assert_eq!(event_count(&db), 0);
    }

    #[test]
    fn batch_failure_rolls_back_events_and_cursor() {
        let mut db = setup();
        let mut bad = sample_event("e-bad");
        bad.tokens.total = Some(i64::MAX as u64 + 1); // 超出 SQLite INTEGER
        let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        let result = commit_batch(
            &mut db,
            run,
            &batch(vec![
                EventInput {
                    event: sample_event("e-good"),
                    replace_existing: false,
                },
                EventInput {
                    event: bad,
                    replace_existing: false,
                },
            ]),
        );
        assert!(result.is_err());
        assert_eq!(event_count(&db), 0, "事件应随事务回滚");
        let cursors: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM ingestion_cursors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cursors, 0, "游标应随事务回滚");
    }

    #[test]
    fn cursor_is_stored_and_updated() {
        let mut db = setup();
        let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        commit_batch(&mut db, run, &batch(vec![])).unwrap();
        let v: String = db
            .conn()
            .query_row(
                "SELECT cursor_value FROM ingestion_cursors WHERE source_id='src1' AND file_identity='fileA'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "1024");

        let mut b2 = batch(vec![]);
        b2.cursor.as_mut().unwrap().cursor_value = "2048".into();
        let run2 = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
        commit_batch(&mut db, run2, &b2).unwrap();
        let v: String = db
            .conn()
            .query_row(
                "SELECT cursor_value FROM ingestion_cursors WHERE source_id='src1' AND file_identity='fileA'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "2048");
    }

    #[test]
    fn schema_has_no_forbidden_secret_columns() {
        // 契约第 9 节：普通表禁止 credential_plaintext / prompt_body / response_body
        let db = Database::open_in_memory().unwrap();
        let mut stmt = db
            .conn()
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        for sql in rows {
            let lower = sql.to_lowercase();
            assert!(!lower.contains("credential_plaintext"), "{lower}");
            assert!(!lower.contains("prompt_body"), "{lower}");
            assert!(!lower.contains("response_body"), "{lower}");
        }
        assert_eq!(migrations::LATEST, 1);
    }
}

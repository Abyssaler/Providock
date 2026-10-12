//! A07 MiniMax Code（MCode）本地运行时 SQLite 用量投影解析器。
//!
//! 证据：docs/research/evidence/a05-a07-ccswitch-importers.md「A07 MCode」节
//! （cc-switch `df40d53c`，`src-tauri/src/services/session_usage_mcode.rs`）。
//! 数据源：`<MINIMAX_DATA_DIR>/v2/sqlite/runtime-state.sqlite` 的单表
//! `local_runtime_token_usage`，列
//! `id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
//! cache_read_tokens, cache_write_tokens, cost_usd`。
//!
//! ## 只读规则（调用方负责打开连接）
//! 上游 cc-switch 以 `SQLITE_OPEN_READ_ONLY` 打开
//! （`session_manager/providers/mcode.rs:13-15`），本适配器沿用并收紧为：
//! - 只读：`OpenFlags::SQLITE_OPEN_READ_ONLY`，绝不打开写；不带 `CREATE`/`READWRITE`。
//! - `busy_timeout`：只读连接遇写锁时等待有限时间（[`READ_ONLY_BUSY_TIMEOUT_MS`]）而非立即失败。
//! - **不**加 `immutable=1`：MCode 运行时的 WAL/checkpoint 行为未获官方说明
//!   （证据第 5 节第 10 条），`immutable` 会让只读连接忽略 `-wal`，可能读到过期快照。
//! - **不动 WAL**：不执行 checkpoint、不读写 `-wal` / `-shm`，交由 SQLite 标准只读路径处理。
//!
//! 便利函数 [`open_read_only`] 实现了上述规则；调用方也可自行以等价 flag 打开连接后调用
//! [`read_events`]。
//!
//! ## 增量游标与重叠回看
//! 上游以 `WHERE id > ?1 ORDER BY id` 取新行并把游标推到最大 id
//! （`session_usage_mcode.rs:36`、`:83`）。但 `id` 是否 append-only **未获官方证实**
//! （证据 3.2 局限、第 5 节第 7 条）：若上游对同一 `id` 补写 usage 或重建库使 id 回退，
//! 纯 `id > cursor` 会漏读。因此本适配器实际读取 `id > max(0, after_id - overlap)`
//! （`overlap` 默认 [`DEFAULT_OVERLAP`] = 8），把游标附近已处理的行重新取出；
//! 重复身份由入库层按 `event_id` 去重，不产生重复计量。`overlap = 0` 时退化为上游行为。
//!
//! ## 映射（保守口径，未确认点见夹具 README）
//! - `input_fresh = input_tokens`（沿用上游「fresh 语义」声明，**待官方样本确认**）
//! - `input_cache_read = cache_read_tokens`；`input_cache_write = cache_write_tokens`
//! - `input_total = 三者之和`；任一分量未知则不求和，保持 `None`（不补零）
//! - `output_total = output_tokens`；`reasoning_tokens` 只进 `output_reasoning`，
//!   **不**另加到 output。上游把两者相加（`session_usage_mcode.rs:49`），但其互斥关系
//!   未经证实，宁可少加也不重复计（见夹具 README 差异说明）。
//! - `total = input_total + output_total`
//! - `model_raw = model` **原样**：不复制上游 `split_once('/')` 丢首段的做法，保留全名，
//!   归一化留给另一层。
//! - v2.0.4 起忽略源 `cost_usd`，不采集自报费用；
//!   `NULL` / 负数 / `NaN` / `Inf` → `None`（不落估算）。
//! - 身份：`event_id = "mcode:{id}"`（原生行 id）；`granularity = request`；
//!   `request_count = 1`（一行一次调用的投影语义）。
//! - `occurred_at = ts` 毫秒 epoch → RFC3339 UTC。上游 `created_at = ts / 1000`
//!   （`session_usage_mcode.rs:79`）即 `ts` 为**毫秒**；**单位是秒还是毫秒待官方样本确认**。
//!   异常/溢出的 `ts` 保持 `None` 并记诊断，不伪造时间。
//!
//! 同库只采一次的规则由调用方保证（`source_instance = 规范化库路径`），解析器不处理。
//!
//! 纯查询函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use rusqlite::{Connection, OpenFlags};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "mcode-sqlite-1";

/// `overlap` 默认回看行数。
pub const DEFAULT_OVERLAP: u64 = 8;

/// 只读连接 `busy_timeout`（毫秒）：遇写锁等待而非立即 `SQLITE_BUSY`。
pub const READ_ONLY_BUSY_TIMEOUT_MS: u64 = 5_000;

pub struct McodeConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    pub observed_at: String,
    /// 增量游标：上轮已处理的最大行 id。`None` = 首次全量读取。
    pub after_id: Option<u64>,
    /// 重叠回看行数；默认 [`DEFAULT_OVERLAP`]。
    pub overlap: u64,
}

impl McodeConfig {
    /// 构造配置，`overlap` 取默认值。
    pub fn new(
        source_id: impl Into<String>,
        file_identity: impl Into<String>,
        generation: u64,
        observed_at: impl Into<String>,
        after_id: Option<u64>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            file_identity: file_identity.into(),
            generation,
            observed_at: observed_at.into(),
            after_id,
            overlap: DEFAULT_OVERLAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum McodeStatus {
    #[default]
    Ok,
    /// 表缺失或列结构不符（真实 DDL 未获官方证实）：标记不可用，不猜字段，保留既有数据。
    Unsupported(String),
    /// 查询执行失败（锁/IO 等）：调用方下一轮重试，不推进游标。
    Deferred(String),
}

#[derive(Debug, Default, Clone)]
pub struct McodeOutcome {
    pub events: Vec<UsageEvent>,
    /// 本次查询观察到的最大行 id（含重叠回看行），不会小于 `after_id`；
    /// 供调用方**单调**推进游标：`new_cursor = max(old_cursor, outcome.max_id)`。
    /// 无任何行且 `after_id = None` 时为 `None`。
    pub max_id: Option<u64>,
    pub diagnostics: Vec<String>,
    pub status: McodeStatus,
}

/// 按只读规则打开 MCode 运行时库（`READ_ONLY` + `busy_timeout`，不 immutable、不动 WAL）。
pub fn open_read_only(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_millis(READ_ONLY_BUSY_TIMEOUT_MS))?;
    Ok(conn)
}

struct RawRow {
    id: i64,
    session_id: Option<String>,
    model: Option<String>,
    ts: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
}

fn map_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        model: row.get(2)?,
        ts: row.get(3)?,
        input_tokens: row.get(4)?,
        output_tokens: row.get(5)?,
        reasoning_tokens: row.get(6)?,
        cache_read_tokens: row.get(7)?,
        cache_write_tokens: row.get(8)?,
    })
}

/// 只读查询 `local_runtime_token_usage`，投影为逻辑计量事件。
/// 连接由调用方按只读规则打开（见模块文档 / [`open_read_only`]）。
pub fn read_events(conn: &Connection, cfg: &McodeConfig) -> McodeOutcome {
    let mut out = McodeOutcome::default();

    let lower: Option<u64> = cfg.after_id.map(|a| a.saturating_sub(cfg.overlap));

    let mut sql = String::from(
        "SELECT id, session_id, model, ts, input_tokens, output_tokens, \
         reasoning_tokens, cache_read_tokens, cache_write_tokens \
         FROM local_runtime_token_usage",
    );
    if lower.is_some() {
        sql.push_str(" WHERE id > ?1");
    }
    sql.push_str(" ORDER BY id");

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            out.status = McodeStatus::Unsupported(format!("prepare failed: {e}"));
            return out;
        }
    };

    let rows_result: rusqlite::Result<Vec<RawRow>> = if let Some(lower) = lower {
        let bound = i64::try_from(lower).unwrap_or(i64::MAX);
        stmt.query_map(rusqlite::params![bound], map_raw)
            .and_then(|it| it.collect())
    } else {
        stmt.query_map(rusqlite::params![], map_raw)
            .and_then(|it| it.collect())
    };
    let rows = match rows_result {
        Ok(rows) => rows,
        Err(e) => {
            out.status = McodeStatus::Deferred(format!("query failed: {e}"));
            out.max_id = cfg.after_id;
            return out;
        }
    };

    let mut max_seen: Option<u64> = None;
    for raw in rows {
        let id = raw.id;
        if id < 0 {
            out.diagnostics
                .push(format!("negative_row_id:{id}: 跳过该行"));
            continue;
        }
        let id_u = id as u64;
        max_seen = Some(max_seen.map_or(id_u, |m| m.max(id_u)));

        let mut row_diag: Vec<String> = Vec::new();
        let fresh = token_nonneg(raw.input_tokens, "input_tokens", id, &mut row_diag);
        let cache_read = token_nonneg(
            raw.cache_read_tokens,
            "cache_read_tokens",
            id,
            &mut row_diag,
        );
        let cache_write = token_nonneg(
            raw.cache_write_tokens,
            "cache_write_tokens",
            id,
            &mut row_diag,
        );
        let output_total = token_nonneg(raw.output_tokens, "output_tokens", id, &mut row_diag);
        let output_reasoning =
            token_nonneg(raw.reasoning_tokens, "reasoning_tokens", id, &mut row_diag);

        let input_total = match (fresh, cache_read, cache_write) {
            (Some(f), Some(r), Some(w)) => match f.checked_add(r).and_then(|s| s.checked_add(w)) {
                Some(t) => Some(t),
                None => {
                    row_diag.push(format!("input_total_overflow:id={id}"));
                    None
                }
            },
            _ => None,
        };

        let total = match (input_total, output_total) {
            (Some(i), Some(o)) => match i.checked_add(o) {
                Some(t) => Some(t),
                None => {
                    row_diag.push(format!("total_overflow:id={id}"));
                    None
                }
            },
            _ => None,
        };

        let occurred_at = raw.ts.and_then(|ts| {
            let at = occurred_at_from_ts_ms(ts);
            if at.is_none() {
                row_diag.push(format!("invalid_ts:id={id}:{ts}"));
            }
            at
        });

        let tokens_all_present = fresh.is_some()
            && cache_read.is_some()
            && cache_write.is_some()
            && output_total.is_some();
        let quality = if fresh.is_some() && output_total.is_some() {
            Quality::Reported
        } else {
            Quality::Unknown
        };
        let completeness = if tokens_all_present {
            Completeness::Complete
        } else {
            Completeness::Partial
        };

        out.events.push(UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: format!("mcode:{id_u}"),
            agent: AgentKind::Mcode,
            source_id: cfg.source_id.clone(),
            stream_domain: StreamDomain::LocalLog,
            session_id: raw.session_id.clone(),
            parent_session_id: None,
            actor_id: None,
            turn_id: None,
            project_ref: None,
            occurred_at,
            observed_at: cfg.observed_at.clone(),
            model_raw: raw.model.clone(),
            model_normalized: None,
            granularity: Granularity::Request,
            request_count: Some(1),
            tokens: TokenSet {
                input_fresh: fresh,
                input_cache_read: cache_read,
                input_cache_write: cache_write,
                input_total,
                output_total,
                output_reasoning,
                total,
                quality,
            },
            cost_estimated: None,
            completeness,
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: cfg.source_id.clone(),
                file_identity: cfg.file_identity.clone(),
                generation: cfg.generation,
                locator: id_u.to_string(),
                parser_version: PARSER_VERSION.into(),
            }],
        });

        out.diagnostics.extend(row_diag);
    }

    out.max_id = match (max_seen, cfg.after_id) {
        (Some(m), Some(a)) => Some(m.max(a)),
        (Some(m), None) => Some(m),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    };

    out
}

fn token_nonneg(v: Option<i64>, col: &str, id: i64, diag: &mut Vec<String>) -> Option<u64> {
    match v {
        None => None,
        Some(n) if n < 0 => {
            diag.push(format!("negative_token:{col}:id={id}:{n}"));
            None
        }
        Some(n) => Some(n as u64),
    }
}

/// `ts` 毫秒 epoch → RFC3339 UTC。异常/溢出返回 `None`（不伪造时间）。
fn occurred_at_from_ts_ms(ts: i64) -> Option<String> {
    let nanos = i128::from(ts).checked_mul(1_000_000)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE local_runtime_token_usage (
               id INTEGER PRIMARY KEY, session_id TEXT, model TEXT, ts INTEGER,
               input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER,
               cache_read_tokens INTEGER, cache_write_tokens INTEGER, cost_usd REAL);",
        )
        .unwrap();
    }

    fn cfg(after_id: Option<u64>, overlap: u64) -> McodeConfig {
        McodeConfig {
            source_id: "test-src".into(),
            file_identity: "fixture".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            after_id,
            overlap,
        }
    }

    #[test]
    fn mcode_maps_reasoning_separately_not_added() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn);
        conn.execute(
            "INSERT INTO local_runtime_token_usage
             (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
              cache_read_tokens, cache_write_tokens, cost_usd)
             VALUES (1, 's1', 'MiniMax-M2', 1791705600078, 100, 20, 7, 30, 5, 0.5)",
            [],
        )
        .unwrap();
        let out = read_events(&conn, &cfg(None, 0));
        assert_eq!(out.status, McodeStatus::Ok);
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!(e.event_id, "mcode:1");
        assert_eq!(e.agent, AgentKind::Mcode);
        assert_eq!(e.granularity, Granularity::Request);
        assert_eq!(e.request_count, Some(1));
        assert_eq!(e.tokens.input_fresh, Some(100));
        assert_eq!(e.tokens.input_cache_read, Some(30));
        assert_eq!(e.tokens.input_cache_write, Some(5));
        assert_eq!(e.tokens.input_total, Some(135));
        // output_total 只取 output_tokens，reasoning 不另加
        assert_eq!(e.tokens.output_total, Some(20));
        assert_eq!(e.tokens.output_reasoning, Some(7));
        assert_eq!(e.tokens.total, Some(155));
        assert_eq!(e.model_raw.as_deref(), Some("MiniMax-M2"));
        assert_eq!(e.occurred_at.as_deref(), Some("2026-10-11T08:00:00.078Z"));
    }

    #[test]
    fn mcode_overlap_relooks_and_max_id_advances() {
        let conn = Connection::open_in_memory().unwrap();
        create_table(&conn);
        for id in 1..=6i64 {
            conn.execute(
                "INSERT INTO local_runtime_token_usage
                 (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
                  cache_read_tokens, cache_write_tokens, cost_usd)
                 VALUES (?1, 's1', 'm', 1791705600000, ?1, 1, 0, 0, 0, NULL)",
                rusqlite::params![id],
            )
            .unwrap();
        }
        // after_id=4, overlap=8 → lower=0，读到全部 6 行（重叠回看）
        let out = read_events(&conn, &cfg(Some(4), 8));
        let ids: Vec<&str> = out.events.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(
            ids,
            ["mcode:1", "mcode:2", "mcode:3", "mcode:4", "mcode:5", "mcode:6"]
        );
        assert_eq!(out.max_id, Some(6));
        // after_id=4, overlap=0 → 只读新增 id>4
        let out2 = read_events(&conn, &cfg(Some(4), 0));
        let ids2: Vec<&str> = out2.events.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids2, ["mcode:5", "mcode:6"]);
        assert_eq!(out2.max_id, Some(6));
        // 无新行时 max_id 不倒退
        let out3 = read_events(&conn, &cfg(Some(6), 0));
        assert!(out3.events.is_empty());
        assert_eq!(out3.max_id, Some(6));
    }

    #[test]
    fn mcode_missing_table_is_unsupported_not_guessed() {
        let conn = Connection::open_in_memory().unwrap();
        let out = read_events(&conn, &cfg(None, 0));
        assert!(matches!(out.status, McodeStatus::Unsupported(_)));
        assert!(out.events.is_empty());
    }

    #[test]
    fn mcode_ts_units_are_milliseconds() {
        assert_eq!(
            occurred_at_from_ts_ms(1791705600000),
            Some("2026-10-11T08:00:00Z".into())
        );
        assert_eq!(occurred_at_from_ts_ms(i64::MAX), None);
    }
}

//! A05 OpenCode 只读 SQLite 用量解析器。
//!
//! 证据：docs/research/evidence/a05-a07-ccswitch-importers.md「A05 OpenCode」节
//! （cc-switch `df40d53c`，`src-tauri/src/services/session_usage_opencode.rs`
//! 与 `src-tauri/src/opencode_config.rs`）。数据源是单个 SQLite 文件
//! （默认 `~/.local/share/opencode/opencode.db`，路径发现由调用方负责）。
//!
//! ## 只读语义（调用方契约，解析器内部复核）
//! 连接由调用方打开；本项目只读 Agent 源，绝不写源库。打开规则：
//! - `OpenFlags::SQLITE_OPEN_READ_ONLY`：不得带 `READ_WRITE` / `CREATE`；
//! - `busy_timeout` **不超过 2s**（[`READ_ONLY_BUSY_TIMEOUT_MS`]）：源库可能正在写，
//!   短暂等待而非无限阻塞或立即 `SQLITE_BUSY`；
//! - **禁止 `immutable=1`**：opencode.db 运行在 WAL 模式，新提交可能仍在
//!   `opencode.db-wal`（证据 1.4），`immutable` 会让只读连接忽略 `-wal`，读到过期快照；
//! - **不 checkpoint、不迁移源库**：V1 表可能被上游冻结（证据 1.2），仍不得写入；
//!   不执行 `PRAGMA wal_checkpoint`，不建表、不改 schema。
//!
//! [`open_read_only`] 实现了上述规则；调用方也可自行以等价 flag 打开后调用
//! [`read_events`]。解析器用 `Connection::is_readonly("main")` 复核，若不可读只
//! 仅记诊断（仍只发 SELECT，不改变只读行为）。
//!
//! ## 版本探测（V2 优先；V1 冻结后不再读）
//! 依据证据 1.2：`detect_schema` 查 `sqlite_master`，**`session_v2` 与
//! `session_message` 同时存在**才判 V2，否则回退 V1（`session` / `message`）。
//! 上游优先规则（证据 1.2 末段）：V2 把会话数据迁移到新表并**冻结 V1 表**，
//! 因此检测到 V2 时**完全不读 V1**，避免双读冻结表；未迁移的库继续走 V1。
//!
//! ## 字段映射（保守口径，不确定点见 fixtures/agents/opencode/README.md）
//! `data` JSON 的 `tokens{input, output, reasoning, cache{read, write}}`：
//! - `input_fresh = input`、`input_cache_read = cache.read`、`input_cache_write = cache.write`
//!   （证据 1.3：Anthropic 风格，input 与 cache 并列，落库时 input 为 fresh）
//! - `input_total = input + cache.read + cache.write`（检查溢出，不补零）
//! - `output_total = output + reasoning`；`output_reasoning = reasoning`。
//!   源 `output` 是否已含 reasoning **未获官方样本确认**（证据第 5 节第 1 条），
//!   本适配器按契约在 reasoning > 0 时相加并标 `quality = derived`；
//!   reasoning 为 0 时 `output_total = output`，`quality = reported`。
//! - `total = input_total + output_total`
//! - 全零 token 行：证据无法区分「源真实写 0」与「未提供」（cc-switch 一律丢弃，
//!   证据 1.3 `:361-369`），按任务要求**按 reported 0 处理**，不丢弃（见 README）。
//!
//! ## 身份与写入语义
//! - `event_id = "opencode:{session_id}:{message_id}"`（原生 id，证据 1.5）。
//! - 消息行会被上游后续更新（证据 1.4/1.5 已确认），**不能只按新增行读取**；
//!   因此每个事件 `replace_existing = true`，调用方重扫时按 `event_id` 整行覆盖。
//! - `granularity = message`、`request_count = 1`（一条 usage 记录一次调用）。
//! - 删除检测不做：V1 冻结后不重复导入即可（任务口径）。
//!
//! ## 时间
//! `occurred_at = data.time.created`，**epoch 毫秒**（证据 1.3 `:386-390`）。
//! 注意上游查询只 SELECT `id, data, type`（证据 1.2），时间只能取自 `data` 内的
//! `time.created`，不是表列 `time_created`（后者仅用于 `ORDER BY`）。缺时间保持 `None`。
//!
//! ## 费用
//! v2.0.4 起不读取源 cost 字段；费用仅由用户配置的模型价格估算。
//!
//! 纯查询函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "opencode-sqlite-1";

/// 只读连接 `busy_timeout`（毫秒）：任务要求 ≤ 2s。
pub const READ_ONLY_BUSY_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCodeSchema {
    /// 旧布局：`session` + `message`（按 `data.role == "assistant"` 过滤）。
    V1,
    /// 新布局：`session_v2` + `session_message`（按 `type IN ('assistant','compaction')` 过滤）。
    V2,
}

impl OpenCodeSchema {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

pub struct OpenCodeConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    /// 本机观察时间（RFC3339）
    pub observed_at: String,
    /// 限定单个会话；`None` = 读取库内全部会话。
    pub session_id: Option<String>,
}

/// 一个逻辑事件 + 写入语义。原生 message ID 身份下，同 id 的新版本替换旧版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    pub event: UsageEvent,
    pub replace_existing: bool,
}

#[derive(Debug, Default)]
pub struct OpenCodeOutcome {
    /// 探测到的布局；库不可读时为 `None`。
    pub schema: Option<OpenCodeSchema>,
    pub events: Vec<ParsedEvent>,
    pub diagnostics: Vec<String>,
}

/// 按只读规则打开 OpenCode 库（`READ_ONLY` + `busy_timeout ≤2s`，不 immutable、不动 WAL）。
pub fn open_read_only(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_millis(READ_ONLY_BUSY_TIMEOUT_MS))?;
    Ok(conn)
}

/// 只读查询 usage 行，投影为逻辑计量事件。连接由调用方按只读规则打开
/// （见模块文档 / [`open_read_only`]）。
pub fn read_events(conn: &Connection, cfg: &OpenCodeConfig) -> OpenCodeOutcome {
    let mut out = OpenCodeOutcome::default();

    // 只读复核：仅诊断，不强制（我们只发 SELECT，不写源库）。
    match conn.is_readonly("main") {
        Ok(true) => {}
        Ok(false) => out.diagnostics.push(
            "connection_not_readonly: 调用方须以 SQLITE_OPEN_READ_ONLY 打开（见模块文档）".into(),
        ),
        Err(e) => out.diagnostics.push(format!("readonly_probe_failed: {e}")),
    }

    let schema = match detect_schema(conn, &mut out) {
        Ok(s) => s,
        Err(e) => {
            out.diagnostics.push(format!("schema_probe_failed: {e}"));
            return out;
        }
    };
    out.schema = Some(schema);
    out.diagnostics.push(format!("schema:{}", schema.as_str()));

    match schema {
        OpenCodeSchema::V1 => read_v1(conn, cfg, &mut out),
        OpenCodeSchema::V2 => read_v2(conn, cfg, &mut out),
    }
    out
}

/// 依据证据 1.2：`session_v2` 与 `session_message` **同时存在**才判 V2，否则 V1。
/// 只有一张 V2 表（迁移中断）时记诊断并按证据回退 V1，不猜。
fn detect_schema(conn: &Connection, out: &mut OpenCodeOutcome) -> rusqlite::Result<OpenCodeSchema> {
    let has_v2_sessions = table_exists(conn, "session_v2")?;
    let has_v2_messages = table_exists(conn, "session_message")?;
    if has_v2_sessions && has_v2_messages {
        return Ok(OpenCodeSchema::V2);
    }
    if has_v2_sessions || has_v2_messages {
        out.diagnostics
            .push("partial_v2_migration: 仅存在一张 V2 表，按证据回退 V1（不猜）".into());
    }
    Ok(OpenCodeSchema::V1)
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

fn read_v1(conn: &Connection, cfg: &OpenCodeConfig, out: &mut OpenCodeOutcome) {
    // 证据 1.2：V1 消息表 message，`SELECT id, data, ... WHERE session_id = ?1
    // ORDER BY time_created`，再按 `data.role == "assistant"` 过滤。
    let sql = "SELECT id, session_id, data FROM message \
               WHERE (?1 IS NULL OR session_id = ?1) ORDER BY time_created";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            out.diagnostics
                .push(format!("v1_query_prepare_failed: {e}"));
            return;
        }
    };
    let rows = match stmt.query_map([cfg.session_id.as_deref()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            out.diagnostics.push(format!("v1_query_failed: {e}"));
            return;
        }
    };
    for row in rows {
        match row {
            Ok((id, session_id, data)) => {
                process_row(&id, &session_id, &data, None, OpenCodeSchema::V1, cfg, out)
            }
            Err(e) => out.diagnostics.push(format!("v1_row_failed: {e}")),
        }
    }
}

fn read_v2(conn: &Connection, cfg: &OpenCodeConfig, out: &mut OpenCodeOutcome) {
    // 证据 1.2：V2 消息表 session_message，按 `type` 过滤（不再看 role）：
    // `type IN ('assistant','compaction')`。
    let sql = "SELECT id, session_id, data, type FROM session_message \
               WHERE (?1 IS NULL OR session_id = ?1) \
                 AND type IN ('assistant','compaction') ORDER BY time_created";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            out.diagnostics
                .push(format!("v2_query_prepare_failed: {e}"));
            return;
        }
    };
    let rows = match stmt.query_map([cfg.session_id.as_deref()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            out.diagnostics.push(format!("v2_query_failed: {e}"));
            return;
        }
    };
    for row in rows {
        match row {
            Ok((id, session_id, data, row_type)) => process_row(
                &id,
                &session_id,
                &data,
                Some(&row_type),
                OpenCodeSchema::V2,
                cfg,
                out,
            ),
            Err(e) => out.diagnostics.push(format!("v2_row_failed: {e}")),
        }
    }
}

/// V1/V2 共用的行处理：解出 `data` JSON、判类型、映射 Token / 费用、生成事件。
fn process_row(
    id: &str,
    session_id: &str,
    data: &str,
    row_type: Option<&str>,
    schema: OpenCodeSchema,
    cfg: &OpenCodeConfig,
    out: &mut OpenCodeOutcome,
) {
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            out.diagnostics.push(format!("bad_message_data:{id}: {e}"));
            return;
        }
    };

    let is_compaction = matches!(schema, OpenCodeSchema::V2) && row_type == Some("compaction");

    if schema == OpenCodeSchema::V1 {
        // V1 只取 assistant；其余（user 等）静默跳过。
        if v.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            return;
        }
    }

    let mut row_diag: Vec<String> = Vec::new();
    let status = if is_compaction {
        v.get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
    } else {
        "assistant"
    };

    let Some(raw) = read_tokens(&v, id, &mut row_diag) else {
        // 证据 1.3：无 tokens 字段的消息不可计（上游同样跳过）。
        let kind = if is_compaction {
            "compaction"
        } else {
            "assistant"
        };
        out.diagnostics
            .push(format!("{kind}_without_tokens:{id}:status={status}"));
        out.diagnostics.extend(row_diag);
        return;
    };

    if is_compaction {
        // 任务口径：compaction 有实际 token 值才计入（completed / failed 都可能有消耗）。
        let has_usage = raw.input != 0
            || raw.output != 0
            || raw.reasoning != 0
            || raw.cache_read != 0
            || raw.cache_write != 0;
        if !has_usage {
            out.diagnostics
                .push(format!("compaction_no_usage:{id}:status={status}"));
            out.diagnostics.extend(row_diag);
            return;
        }
        out.diagnostics
            .push(format!("compaction_counted:{id}:status={status}"));
    }

    let input_total = raw
        .input
        .checked_add(raw.cache_read)
        .and_then(|s| s.checked_add(raw.cache_write));
    let Some(input_total) = input_total else {
        row_diag.push(format!("input_total_overflow:{id}"));
        out.diagnostics.extend(row_diag);
        return;
    };
    let Some(output_total) = raw.output.checked_add(raw.reasoning) else {
        row_diag.push(format!("output_total_overflow:{id}"));
        out.diagnostics.extend(row_diag);
        return;
    };
    let total = match input_total.checked_add(output_total) {
        Some(t) => Some(t),
        None => {
            row_diag.push(format!("total_overflow:{id}"));
            None
        }
    };

    // reasoning > 0 表示 output_total 由「可见输出 + reasoning」相加而得 → derived。
    let quality = if raw.reasoning > 0 {
        Quality::Derived
    } else {
        Quality::Reported
    };

    let occurred_at = match parse_occurred_at(&v) {
        Some(at) => Some(at),
        None => {
            row_diag.push(format!("missing_or_invalid_time:{id}"));
            None
        }
    };

    let event = UsageEvent {
        schema_version: USAGE_EVENT_SCHEMA_VERSION,
        event_id: format!("opencode:{session_id}:{id}"),
        agent: AgentKind::Opencode,
        source_id: cfg.source_id.clone(),
        stream_domain: StreamDomain::LocalLog,
        session_id: Some(session_id.to_string()),
        parent_session_id: None,
        actor_id: None,
        turn_id: None,
        project_ref: None,
        occurred_at,
        observed_at: cfg.observed_at.clone(),
        model_raw: model_id(&v, schema),
        model_normalized: None,
        granularity: Granularity::Message,
        request_count: Some(1),
        tokens: TokenSet {
            input_fresh: Some(raw.input),
            input_cache_read: Some(raw.cache_read),
            input_cache_write: Some(raw.cache_write),
            input_total: Some(input_total),
            output_total: Some(output_total),
            output_reasoning: Some(raw.reasoning),
            total,
            quality,
        },
        cost_estimated: None,
        completeness: Completeness::Complete,
        account_link: None,
        evidence_refs: vec![EvidenceRef {
            source_id: cfg.source_id.clone(),
            file_identity: cfg.file_identity.clone(),
            generation: cfg.generation,
            locator: format!("{session_id}:{id}"),
            parser_version: PARSER_VERSION.into(),
        }],
    };

    out.events.push(ParsedEvent {
        event,
        // 消息行会被上游后续更新，重扫必须整行覆盖（证据 1.4/1.5）。
        replace_existing: true,
    });
    out.diagnostics.extend(row_diag);
}

struct RawTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
}

/// 要求 `tokens` 为对象；缺字段或非无符号整数一律按 0（证据口径；见 README 的决定）。
fn read_tokens(v: &Value, id: &str, diag: &mut Vec<String>) -> Option<RawTokens> {
    let tokens = v.get("tokens")?;
    if !tokens.is_object() {
        return None;
    }
    let cache = tokens.get("cache");
    Some(RawTokens {
        input: token_u64(tokens.get("input"), "tokens.input", id, diag),
        output: token_u64(tokens.get("output"), "tokens.output", id, diag),
        reasoning: token_u64(tokens.get("reasoning"), "tokens.reasoning", id, diag),
        cache_read: token_u64(
            cache.and_then(|c| c.get("read")),
            "tokens.cache.read",
            id,
            diag,
        ),
        cache_write: token_u64(
            cache.and_then(|c| c.get("write")),
            "tokens.cache.write",
            id,
            diag,
        ),
    })
}

/// 单个 token 字段：缺失或非无符号整数一律按 0（证据口径；见 README 的决定）。
fn token_u64(x: Option<&Value>, key: &str, id: &str, diag: &mut Vec<String>) -> u64 {
    match x {
        None => 0,
        Some(x) => match x.as_u64() {
            Some(n) => n,
            None => {
                diag.push(format!("non_unsigned_token:{key}:{id}:{x}"));
                0
            }
        },
    }
}

/// 证据 1.3：`data.time.created` 为 **epoch 毫秒**。缺字段/非法值 → `None`。
fn parse_occurred_at(v: &Value) -> Option<String> {
    let ms = v.get("time")?.get("created")?.as_i64()?;
    let nanos = i128::from(ms).checked_mul(1_000_000)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
}

/// 证据 1.3/1.6：`data.cost` 为聚合 f64。有值且有限、非负 → USD reported（0 保留）。
/// 证据 1.3：V1 `modelID`（字符串）；V2 `model.id` 或 `model` 字符串。
fn model_id(v: &Value, schema: OpenCodeSchema) -> Option<String> {
    match schema {
        OpenCodeSchema::V1 => v
            .get("modelID")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        OpenCodeSchema::V2 => {
            let m = v.get("model")?;
            if let Some(s) = m.as_str() {
                return Some(s.to_string());
            }
            m.get("id").and_then(|x| x.as_str()).map(str::to_string)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1_DDL: &str = "
        CREATE TABLE session (id TEXT PRIMARY KEY, time_created INTEGER, time_updated INTEGER);
        CREATE TABLE message (
            id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
    ";

    const V2_DDL: &str = "
        CREATE TABLE session_v2 (id TEXT PRIMARY KEY, time_created INTEGER, time_updated INTEGER);
        CREATE TABLE session_message (
            id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
            time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
    ";

    fn cfg() -> OpenCodeConfig {
        OpenCodeConfig {
            source_id: "test-src".into(),
            file_identity: "fixture".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            session_id: None,
        }
    }

    fn insert_v1(conn: &Connection, id: &str, session_id: &str, data: &str) {
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data)
             VALUES (?1, ?2, 0, 0, ?3)",
            rusqlite::params![id, session_id, data],
        )
        .unwrap();
    }

    fn insert_v2(conn: &Connection, id: &str, session_id: &str, ty: &str, data: &str) {
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, 0, 0, ?4)",
            rusqlite::params![id, session_id, ty, data],
        )
        .unwrap();
    }

    #[test]
    fn v1_maps_input_output_reasoning_and_cache() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_DDL).unwrap();
        insert_v1(
            &conn,
            "m1",
            "s1",
            r#"{"role":"assistant","modelID":"claude-sonnet-4","cost":0.05,
                "tokens":{"input":1000,"output":100,"reasoning":20,"cache":{"read":300,"write":50}},
                "time":{"created":1760000000000,"completed":1760000001000}}"#,
        );
        insert_v1(&conn, "m2", "s1", r#"{"role":"user","tokens":{"input":9}}"#);
        let out = read_events(&conn, &cfg());
        assert_eq!(out.schema, Some(OpenCodeSchema::V1));
        assert_eq!(out.events.len(), 1, "只取 assistant");
        let e = &out.events[0].event;
        assert_eq!(e.event_id, "opencode:s1:m1");
        assert_eq!(e.agent, AgentKind::Opencode);
        assert_eq!(e.granularity, Granularity::Message);
        assert_eq!(e.request_count, Some(1));
        assert_eq!(e.tokens.input_fresh, Some(1000));
        assert_eq!(e.tokens.input_cache_read, Some(300));
        assert_eq!(e.tokens.input_cache_write, Some(50));
        assert_eq!(e.tokens.input_total, Some(1350));
        assert_eq!(e.tokens.output_total, Some(120));
        assert_eq!(e.tokens.output_reasoning, Some(20));
        assert_eq!(e.tokens.total, Some(1470));
        assert_eq!(e.tokens.quality, Quality::Derived);
        assert_eq!(e.model_raw.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(e.occurred_at.as_deref(), Some("2025-10-09T08:53:20Z"));
        assert!(out.events[0].replace_existing);
    }

    #[test]
    fn v2_prefers_over_v1_and_reads_compaction() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_DDL).unwrap();
        conn.execute_batch(V2_DDL).unwrap();
        // 冻结的 V1 行必须被忽略
        insert_v1(
            &conn,
            "old",
            "s-frozen",
            r#"{"role":"assistant","tokens":{"input":9999}}"#,
        );
        insert_v2(
            &conn,
            "a1",
            "s2",
            "assistant",
            r#"{"model":{"id":"claude-opus-4"},"cost":1.25,
                "tokens":{"input":500,"output":40,"reasoning":10,"cache":{"read":100,"write":20}}}"#,
        );
        insert_v2(&conn, "u1", "s2", "user", r#"{"tokens":{"input":7}}"#);
        insert_v2(
            &conn,
            "c1",
            "s2",
            "compaction",
            r#"{"status":"completed","tokens":{"input":300,"reasoning":50}}"#,
        );
        insert_v2(&conn, "c2", "s2", "compaction", r#"{"status":"failed"}"#);
        let out = read_events(&conn, &cfg());
        assert_eq!(out.schema, Some(OpenCodeSchema::V2));
        let ids: Vec<&str> = out
            .events
            .iter()
            .map(|p| p.event.event_id.as_str())
            .collect();
        assert_eq!(ids, ["opencode:s2:a1", "opencode:s2:c1"]);
        assert_eq!(
            out.events[0].event.model_raw.as_deref(),
            Some("claude-opus-4")
        );
        assert_eq!(out.events[1].event.tokens.input_total, Some(300));
        assert_eq!(out.events[1].event.tokens.output_total, Some(50));
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.contains("compaction_without_tokens:c2")),
            "{:?}",
            out.diagnostics
        );
    }

    #[test]
    fn all_zero_tokens_reported_zero_not_dropped() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_DDL).unwrap();
        insert_v1(
            &conn,
            "z",
            "s1",
            r#"{"role":"assistant","tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}"#,
        );
        let out = read_events(&conn, &cfg());
        assert_eq!(out.events.len(), 1);
        let t = &out.events[0].event.tokens;
        assert_eq!(t.input_total, Some(0));
        assert_eq!(t.output_total, Some(0));
        assert_eq!(t.total, Some(0));
        assert_eq!(t.quality, Quality::Reported);
    }

    #[test]
    fn reasoning_zero_keeps_reported_quality() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_DDL).unwrap();
        insert_v1(
            &conn,
            "m",
            "s1",
            r#"{"role":"assistant","tokens":{"input":10,"output":5,"reasoning":0}}"#,
        );
        let out = read_events(&conn, &cfg());
        assert_eq!(out.events[0].event.tokens.output_total, Some(5));
        assert_eq!(out.events[0].event.tokens.quality, Quality::Reported);
    }

    #[test]
    fn session_filter_limits_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_DDL).unwrap();
        insert_v1(
            &conn,
            "a",
            "s1",
            r#"{"role":"assistant","tokens":{"input":1}}"#,
        );
        insert_v1(
            &conn,
            "b",
            "s2",
            r#"{"role":"assistant","tokens":{"input":2}}"#,
        );
        let mut c = cfg();
        c.session_id = Some("s2".into());
        let out = read_events(&conn, &c);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].event.event_id, "opencode:s2:b");
    }

    #[test]
    fn time_is_epoch_milliseconds() {
        let v: Value = serde_json::from_str(r#"{"time":{"created":1760000000000}}"#).unwrap();
        assert_eq!(
            parse_occurred_at(&v).as_deref(),
            Some("2025-10-09T08:53:20Z")
        );
        let bad: Value = serde_json::from_str(r#"{"time":{}}"#).unwrap();
        assert_eq!(parse_occurred_at(&bad), None);
    }
}

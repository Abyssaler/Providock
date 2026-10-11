//! 版本化迁移。schema_version 与 adapter/parser_version 分开管理（开发手册第 7 节）。
//! 迁移在事务中执行；升级前的备份策略由应用层（打开 DB 时）负责。

use super::{Result, StorageError};
use rusqlite::Connection;

pub const LATEST: u32 = 1;

const MIGRATION_0001: &str = r#"
-- 采集源实例：一个 Agent 的一个根目录/数据库为一个 source
CREATE TABLE sources (
    source_id      TEXT PRIMARY KEY,
    agent          TEXT NOT NULL,
    stream_domain  TEXT NOT NULL,
    root_path      TEXT NOT NULL,          -- 本机路径，展示层可脱敏
    display_label  TEXT,
    enabled        INTEGER NOT NULL DEFAULT 1,
    compat_status  TEXT NOT NULL DEFAULT 'unknown',
    parser_version TEXT NOT NULL,
    first_record_at TEXT,
    last_sync_at    TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- 源文件身份：规范化身份 + 代数（轮转/替换递增）
CREATE TABLE source_files (
    file_id       INTEGER PRIMARY KEY,
    source_id     TEXT NOT NULL REFERENCES sources(source_id) ON DELETE CASCADE,
    file_identity TEXT NOT NULL,
    generation    INTEGER NOT NULL DEFAULT 0,
    path          TEXT NOT NULL,
    size_bytes    INTEGER,
    mtime         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    UNIQUE (source_id, file_identity, generation)
);

-- 事务游标：与事件 upsert 同事务提交
CREATE TABLE ingestion_cursors (
    source_id      TEXT NOT NULL REFERENCES sources(source_id) ON DELETE CASCADE,
    file_identity  TEXT NOT NULL,
    generation     INTEGER NOT NULL,
    parser_version TEXT NOT NULL,
    cursor_kind    TEXT NOT NULL,          -- byte_offset / row_id / composite
    cursor_value   TEXT NOT NULL,
    tail_checksum  TEXT,
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (source_id, file_identity, generation)
);

-- 解析运行：解析器升级通过新 parse_run 重建 + 事务切换活跃版本
CREATE TABLE parse_runs (
    run_id         INTEGER PRIMARY KEY,
    source_id      TEXT NOT NULL REFERENCES sources(source_id) ON DELETE CASCADE,
    parser_version TEXT NOT NULL,
    started_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    finished_at    TEXT,
    status         TEXT NOT NULL DEFAULT 'running',  -- running / committed / failed
    is_active      INTEGER NOT NULL DEFAULT 0,
    events_written INTEGER NOT NULL DEFAULT 0,
    error          TEXT
);

CREATE TABLE sessions (
    session_key    TEXT PRIMARY KEY,       -- source_id + 原生 session_id
    source_id      TEXT NOT NULL REFERENCES sources(source_id) ON DELETE CASCADE,
    agent          TEXT NOT NULL,
    session_id     TEXT NOT NULL,
    parent_session_key TEXT,
    title          TEXT,                   -- 可能敏感，展示可隐藏
    project_ref    TEXT,
    first_seen_at  TEXT,
    last_seen_at   TEXT,
    archived       INTEGER NOT NULL DEFAULT 0,
    has_subagents  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_sessions_source ON sessions(source_id);

CREATE TABLE actors (
    session_key TEXT NOT NULL REFERENCES sessions(session_key) ON DELETE CASCADE,
    actor_id    TEXT NOT NULL,
    role        TEXT,                      -- main / subagent 等，由适配器给
    PRIMARY KEY (session_key, actor_id)
);

CREATE TABLE usage_events (
    event_id         TEXT PRIMARY KEY,     -- 逻辑事件稳定身份
    schema_version   INTEGER NOT NULL,
    parse_run_id     INTEGER NOT NULL REFERENCES parse_runs(run_id),
    source_id        TEXT NOT NULL REFERENCES sources(source_id) ON DELETE CASCADE,
    agent            TEXT NOT NULL,
    stream_domain    TEXT NOT NULL,
    session_id       TEXT,
    parent_session_id TEXT,
    actor_id         TEXT,
    turn_id          TEXT,
    project_ref      TEXT,
    occurred_at      TEXT,                 -- 未知保持 NULL，不伪造
    observed_at      TEXT NOT NULL,
    model_raw        TEXT,
    model_normalized TEXT,
    granularity      TEXT NOT NULL,
    request_count    INTEGER,              -- 未知为 NULL
    input_fresh      INTEGER,
    input_cache_read INTEGER,
    input_cache_write INTEGER,
    input_total      INTEGER,
    output_total     INTEGER,
    output_reasoning INTEGER,
    total            INTEGER,
    tokens_quality   TEXT NOT NULL DEFAULT 'reported',
    completeness     TEXT NOT NULL DEFAULT 'unknown',
    cost_reported_amount   TEXT,
    cost_reported_currency TEXT,
    cost_reported_completeness TEXT,
    cost_estimated_amount   TEXT,
    cost_estimated_currency TEXT,
    cost_estimated_price_version TEXT,
    account_link     TEXT,                 -- JSON：AccountLink
    evidence_json    TEXT                  -- JSON：EvidenceRef[]
);
CREATE INDEX idx_events_occurred ON usage_events(occurred_at);
CREATE INDEX idx_events_source ON usage_events(source_id);
CREATE INDEX idx_events_agent ON usage_events(agent);
CREATE INDEX idx_events_model ON usage_events(model_normalized);
CREATE INDEX idx_events_session ON usage_events(source_id, session_id);

-- 物理观察：一个逻辑事件可由多条观察支持（流式更新、重扫）
CREATE TABLE usage_observations (
    observation_id INTEGER PRIMARY KEY,
    event_id       TEXT NOT NULL REFERENCES usage_events(event_id) ON DELETE CASCADE,
    source_id      TEXT NOT NULL,
    file_identity  TEXT NOT NULL,
    generation     INTEGER NOT NULL,
    locator        TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    observed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE (event_id, source_id, file_identity, generation, locator)
);

-- 身份墓碑：明细清理后旧文件不会再次计入
CREATE TABLE event_identity_tombstones (
    event_id   TEXT PRIMARY KEY,
    retired_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    reason     TEXT
);

-- ===== 账户领域（与用量账本完全分域） =====

CREATE TABLE accounts (
    account_id    TEXT PRIMARY KEY,
    platform      TEXT NOT NULL,
    region        TEXT,
    display_name  TEXT NOT NULL,
    vendor_user_id TEXT,
    scope         TEXT,                    -- organization / project 等
    config_kind   TEXT NOT NULL,           -- api_payg / coding_plan / both
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    deleted_at    TEXT
);

-- 凭据只保存引用、类型、掩码与本机带盐 HMAC 指纹；明文只在系统凭据库
CREATE TABLE credential_refs (
    credential_id TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    credential_type TEXT NOT NULL,         -- api_key / management_key / oauth_handle / local_server_token / ak_sk
    mask          TEXT NOT NULL,
    fingerprint   TEXT NOT NULL,
    vault_ref     TEXT NOT NULL,           -- 系统凭据库条目名
    capabilities  TEXT,                    -- JSON
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    retired_at    TEXT
);

-- 用户确认的钱包共享；未确认不因余额相同而合并
CREATE TABLE wallet_links (
    wallet_id   TEXT NOT NULL,
    account_id  TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    confirmed   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (wallet_id, account_id)
);

CREATE TABLE account_snapshots (
    snapshot_id    INTEGER PRIMARY KEY,
    account_id     TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    credential_id  TEXT REFERENCES credential_refs(credential_id),
    queried_at     TEXT NOT NULL,          -- 本机请求时间
    provider_as_of TEXT,                   -- 平台数据时间，可能为 NULL
    status         TEXT NOT NULL,          -- ok / stale / refreshing / auth_required / rate_limited / unsupported / incompatible / unavailable
    evidence       TEXT,                   -- 接口版本与来源标注
    error_code     TEXT,
    expires_at     TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_snapshots_account ON account_snapshots(account_id, queried_at);

-- 快照内的余额条目；不同币种/单位分开成行
CREATE TABLE balances (
    snapshot_id  INTEGER NOT NULL REFERENCES account_snapshots(snapshot_id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,            -- paid / grant / available / credit
    amount       TEXT NOT NULL,            -- 十进制字符串
    currency     TEXT NOT NULL,
    wallet_scope TEXT,
    expires_at   TEXT,
    formula      TEXT,                     -- 来源公式说明（如 total = paid + grant）
    quality      TEXT NOT NULL DEFAULT 'reported'
);

CREATE TABLE quota_windows (
    window_id    TEXT NOT NULL,            -- 平台+计划+scope+资源组+窗口标识
    snapshot_id  INTEGER NOT NULL REFERENCES account_snapshots(snapshot_id) ON DELETE CASCADE,
    label        TEXT,
    resource     TEXT,
    scope        TEXT,
    unit         TEXT NOT NULL,            -- tokens / requests / credits / currency / percent / vendor_unit
    period_kind  TEXT,                     -- rolling / calendar / fixed / unknown
    duration_seconds INTEGER,              -- 无证据保持 NULL，不推断
    starts_at    TEXT,
    resets_at    TEXT,
    used         TEXT,
    limit_value  TEXT,
    remaining    TEXT,
    used_ratio      TEXT,
    remaining_ratio TEXT,
    raw_ratio       TEXT,
    over_limit   INTEGER,
    quality      TEXT NOT NULL DEFAULT 'reported',
    evidence_ref TEXT,
    PRIMARY KEY (window_id, snapshot_id)  -- 同一窗口的每次快照各存一行，保留历史
);
CREATE INDEX idx_quota_snapshot ON quota_windows(snapshot_id);

-- 快照内的账户级用量指标（DATA_CONTRACTS 第 7 节 usage_metrics[]）。
-- 值用十进制字符串，unit/quality 与余额、窗口同一套契约取值；scope 可空。
CREATE TABLE usage_metrics (
    snapshot_id INTEGER NOT NULL REFERENCES account_snapshots(snapshot_id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    value       TEXT NOT NULL,             -- 十进制字符串
    unit        TEXT NOT NULL,             -- tokens / requests / credits / currency / percent / vendor_unit
    scope       TEXT,                      -- account / organization / project / key / subscription
    quality     TEXT NOT NULL DEFAULT 'reported',
    PRIMARY KEY (snapshot_id, name, scope)
);
CREATE INDEX idx_usage_metrics_snapshot ON usage_metrics(snapshot_id);

CREATE TABLE price_versions (
    price_version TEXT NOT NULL,
    provider      TEXT NOT NULL,
    model_version TEXT NOT NULL,
    currency      TEXT NOT NULL,
    effective_from TEXT,
    effective_to   TEXT,
    region        TEXT,
    tier          TEXT,                    -- 长上下文档位等
    rates_json    TEXT NOT NULL,           -- input/output/cache read/cache write/TTL 差异
    PRIMARY KEY (price_version, provider, model_version)
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- 结构化诊断：只有适配器、错误码、匿名文件 ID 与计数，不含路径/密钥/正文
CREATE TABLE diagnostic_events (
    id         INTEGER PRIMARY KEY,
    at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    component  TEXT NOT NULL,
    error_code TEXT NOT NULL,
    file_anon  TEXT,
    count      INTEGER,
    detail     TEXT
);
"#;

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    if version >= LATEST {
        return Ok(());
    }
    let tx = conn.transaction().map_err(StorageError::Sqlite)?;
    if version < 1 {
        tx.execute_batch(MIGRATION_0001)
            .map_err(|e| StorageError::Migration {
                version: 1,
                source: e,
            })?;
    }
    tx.pragma_update(None, "user_version", LATEST)
        .map_err(StorageError::Sqlite)?;
    tx.commit().map_err(StorageError::Sqlite)?;
    Ok(())
}

pub fn current_version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

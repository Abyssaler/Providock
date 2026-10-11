//! T13 会话查询与可选正文查看的端到端夹具测试（临时源树 + 自有 DB）。
//!
//! 覆盖：
//! - `sessions_page`：Token 小计（Rust u128 → 十进制字符串）、事件数、最后活跃、键集游标分页；
//! - `session_content_page`：开关默认关闭 → `ContentDisabled`；开启后按源规则定位并只读读取；
//!   源文件缺失 → `SourceGone`；未实现读取器的源 → `UnsupportedSource`；形状不符 → `Incompatible`；
//!   身份含路径段 → `PathRejected`；
//! - `content_view_set` / `content_view_enabled` 的持久化。
//!
//! 夹具：`fixtures/content/kimi_code/wire-human/input/wire.jsonl`（仓库内合成数据，非真实日志）。

use providock_lib::services::session_queries::{
    content_view_enabled, content_view_set, session_content_page, sessions_page, ContentError,
    SessionFilter, SessionRef, CONTENT_VIEW_SETTING_KEY,
};
use providock_lib::storage::Database;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(
            "providock-t13-{tag}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn insert_source(db: &Database, source_id: &str, agent: &str, root: &Path) {
    db.conn()
        .execute(
            "INSERT INTO sources (source_id, agent, stream_domain, root_path, parser_version)
             VALUES (?1, ?2, 'local_log', ?3, 'test')",
            rusqlite::params![source_id, agent, root.to_string_lossy()],
        )
        .unwrap();
}

fn insert_session(
    db: &Database,
    source_id: &str,
    agent: &str,
    session_id: &str,
    last_seen: Option<&str>,
) {
    db.conn()
        .execute(
            "INSERT INTO sessions (session_key, source_id, agent, session_id, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                format!("{source_id}:{session_id}"),
                source_id,
                agent,
                session_id,
                last_seen
            ],
        )
        .unwrap();
}

fn insert_event(
    db: &Database,
    run_id: i64,
    source_id: &str,
    session_id: &str,
    at: &str,
    total: i64,
) {
    db.conn()
        .execute(
            "INSERT INTO usage_events (event_id, schema_version, parse_run_id, source_id, agent,
                stream_domain, session_id, occurred_at, observed_at, granularity, total, model_normalized,
                tokens_quality, completeness)
             VALUES (?1, 1, ?2, ?3, 'kimi_code', 'local_log', ?4, ?5, ?5, 'request', ?6, 'synthetic-model',
                     'reported', 'complete')",
            rusqlite::params![
                format!("evt:{source_id}:{session_id}:{at}"),
                run_id,
                source_id,
                session_id,
                at,
                total
            ],
        )
        .unwrap();
}

fn insert_run(db: &Database, source_id: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO parse_runs (source_id, parser_version, status, is_active)
             VALUES (?1, 'test', 'committed', 1)",
            [source_id],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

/// 造一棵 Kimi 源树，wire 正文取自仓库合成夹具。
fn kimi_tree(dir: &TempDir) -> PathBuf {
    let target = dir
        .path()
        .join("sessions/wd-key/session-alpha/agents/main/wire.jsonl");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::copy(
        fixture("content/kimi_code/wire-human/input/wire.jsonl"),
        &target,
    )
    .unwrap();
    target
}

#[test]
fn sessions_page_aggregates_tokens_and_pages_with_keyset_cursor() {
    let dir = TempDir::new("list");
    let db = Database::open_in_memory().unwrap();
    insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
    insert_source(&db, "pi:default", "pi", dir.path());
    insert_session(&db, "kimi_code:default", "kimi_code", "session-alpha", None);
    insert_session(&db, "kimi_code:default", "kimi_code", "session-beta", None);
    insert_session(&db, "pi:default", "pi", "pi-session", None);

    let run = insert_run(&db, "kimi_code:default");
    insert_event(
        &db,
        run,
        "kimi_code:default",
        "session-alpha",
        "2026-10-10T00:00:00Z",
        i64::MAX,
    );
    insert_event(
        &db,
        run,
        "kimi_code:default",
        "session-alpha",
        "2026-10-10T01:00:00Z",
        3,
    );
    insert_event(
        &db,
        run,
        "kimi_code:default",
        "session-beta",
        "2026-10-09T00:00:00Z",
        7,
    );

    let page = sessions_page(&db, &SessionFilter::default(), None, 2).unwrap();
    assert_eq!(page.items.len(), 2, "每页 2 条");
    assert!(page.has_more);
    assert_eq!(page.items[0].session_id, "session-alpha");
    assert_eq!(page.items[0].event_count, 2);
    assert_eq!(
        page.items[0].last_active_at.as_deref(),
        Some("2026-10-10T01:00:00Z")
    );
    // i64::MAX + 3 不溢出，且以十进制字符串返回
    assert_eq!(
        page.items[0].token_total.as_deref(),
        Some("9223372036854775810")
    );
    assert_eq!(page.items[0].token_known_events, 2);
    assert_eq!(page.items[0].models, vec!["synthetic-model"]);
    // 列表项本身不含任何本机绝对路径
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains(&dir.path().to_string_lossy().to_string()));

    let page2 = sessions_page(
        &db,
        &SessionFilter::default(),
        page.next_cursor.as_deref(),
        2,
    )
    .unwrap();
    assert_eq!(page2.items.len(), 1);
    assert!(!page2.has_more);
    // 无事件、无 last_seen_at 的会话排在最后且小计为 null（不是 0）
    assert_eq!(page2.items[0].session_id, "pi-session");
    assert_eq!(page2.items[0].token_total, None);
    assert_eq!(page2.items[0].last_active_at, None);

    // Agent 筛选
    let only_pi = sessions_page(
        &db,
        &SessionFilter {
            agent: Some("pi".into()),
            ..Default::default()
        },
        None,
        10,
    )
    .unwrap();
    assert_eq!(only_pi.items.len(), 1);
    assert_eq!(only_pi.items[0].agent, "pi");

    // 元数据搜索（session_id）；大小写不敏感
    let found = sessions_page(
        &db,
        &SessionFilter {
            query: Some("BETA".into()),
            ..Default::default()
        },
        None,
        10,
    )
    .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].session_id, "session-beta");

    // 时间区间半开 [from, to)
    let ranged = sessions_page(
        &db,
        &SessionFilter {
            from: Some("2026-10-10T00:00:00Z".into()),
            to: Some("2026-10-11T00:00:00Z".into()),
            ..Default::default()
        },
        None,
        10,
    )
    .unwrap();
    assert_eq!(ranged.items.len(), 1);
    assert_eq!(ranged.items[0].session_id, "session-alpha");
}

#[test]
fn content_view_is_off_by_default_and_then_reads_source_on_demand() {
    let dir = TempDir::new("content");
    let wire = kimi_tree(&dir);
    let db = Database::open_in_memory().unwrap();
    insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
    insert_session(&db, "kimi_code:default", "kimi_code", "session-alpha", None);

    let session_ref = SessionRef {
        source_id: "kimi_code:default".into(),
        session_id: "session-alpha".into(),
        actor_id: None,
    };

    // 默认关闭：拒绝读取，且不产生任何正文存储
    assert!(!content_view_enabled(&db).unwrap());
    assert_eq!(
        session_content_page(&db, &session_ref, None, 10).unwrap_err(),
        ContentError::ContentDisabled
    );
    let setting_rows: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = ?1",
            [CONTENT_VIEW_SETTING_KEY],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(setting_rows, 0, "默认关闭时不写设置行");

    // 开启后按需读取源文件并分页
    assert!(content_view_set(&db, true).unwrap());
    let page = session_content_page(&db, &session_ref, None, 2).unwrap();
    assert_eq!(page.agent, "kimi_code");
    assert_eq!(page.blocks.len(), 2);
    assert_eq!(page.blocks[0].role, "user");
    assert!(page.has_more);
    assert!(wire.is_file(), "只读读取不改动源文件");

    let cursor: u64 = page.next_cursor.as_deref().unwrap().parse().unwrap();
    let page2 = session_content_page(&db, &session_ref, Some(cursor), 2).unwrap();
    assert!(!page2.blocks.is_empty());
    // 正文未落任何表：usage_events / sessions 之外没有正文列可写（schema 检查）
    let columns: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM pragma_table_info('usage_events')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!columns
        .iter()
        .any(|c| c.contains("prompt") || c.contains("body") || c.contains("content")));
}

#[test]
fn content_reports_missing_unsupported_incompatible_and_rejected() {
    let dir = TempDir::new("reasons");
    let db = Database::open_in_memory().unwrap();
    insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
    insert_source(&db, "pi:default", "pi", dir.path());
    insert_session(&db, "kimi_code:default", "kimi_code", "session-alpha", None);
    insert_session(&db, "pi:default", "pi", "pi-session", None);
    content_view_set(&db, true).unwrap();

    // 源文件缺失
    assert_eq!(
        session_content_page(
            &db,
            &SessionRef {
                source_id: "kimi_code:default".into(),
                session_id: "session-alpha".into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap_err(),
        ContentError::SourceGone
    );

    // 未实现读取器的源：明确原因，不猜路径
    let unsupported = session_content_page(
        &db,
        &SessionRef {
            source_id: "pi:default".into(),
            session_id: "pi-session".into(),
            actor_id: None,
        },
        None,
        10,
    )
    .unwrap_err();
    assert_eq!(unsupported.code(), "UnsupportedSource");

    // 形状不符（Kimi wire 首行不是 metadata）
    let bad = dir
        .path()
        .join("sessions/wd/session-bad/agents/main/wire.jsonl");
    std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
    std::fs::write(&bad, "{\"type\":\"llm.request\",\"agentId\":\"main\"}\n").unwrap();
    insert_session(&db, "kimi_code:default", "kimi_code", "session-bad", None);
    let incompatible = session_content_page(
        &db,
        &SessionRef {
            source_id: "kimi_code:default".into(),
            session_id: "session-bad".into(),
            actor_id: None,
        },
        None,
        10,
    )
    .unwrap_err();
    assert_eq!(incompatible.code(), "Incompatible");

    // 身份含路径段：拒绝定位，不接触文件系统
    let rejected = session_content_page(
        &db,
        &SessionRef {
            source_id: "kimi_code:default".into(),
            session_id: "../../etc/passwd".into(),
            actor_id: None,
        },
        None,
        10,
    )
    .unwrap_err();
    assert_eq!(rejected.code(), "PathRejected");

    // 索引外的会话：SessionNotFound（即使文件存在也不读）
    let stray = dir
        .path()
        .join("sessions/wd/not-indexed/agents/main/wire.jsonl");
    std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
    std::fs::copy(
        fixture("content/kimi_code/wire-human/input/wire.jsonl"),
        &stray,
    )
    .unwrap();
    let not_indexed = session_content_page(
        &db,
        &SessionRef {
            source_id: "kimi_code:default".into(),
            session_id: "not-indexed".into(),
            actor_id: None,
        },
        None,
        10,
    )
    .unwrap_err();
    assert_eq!(not_indexed.code(), "SessionNotFound");
}

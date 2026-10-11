//! T05–T07 扫描器适配器集成测试：A01/A03/A04/A05/A06/A07 的
//! 发现 → 游标 → parse_run → commit_batch 全链路。
//!
//! 每个用例在临时目录（`std::env::temp_dir()` + 唯一名，测试后清理）构造最小目录树，
//! 输入直接取自 `fixtures/agents/` 的合成夹具。断言：
//! 1) 首扫导入事件且聚合总量 == 夹具 `expected.json` 的 `totals`（或就地硬编码 + 算术注释）；
//! 2) 二次扫描 `events_inserted == 0`（重扫幂等）；
//! 3) 追加/改写后只增量新增或校正，不双算。

use providock_lib::adapters::agents::pi_log;
use providock_lib::adapters::agents::{claude_code, gemini_cli, grok_build, mcode_db, opencode_db};
use providock_lib::domain::AgentKind;
use providock_lib::ingestion::scanner::{scan_source, DiscoveredSource};
use providock_lib::storage::Database;
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents")
}

/// 独占临时目录，Drop 时递归清理。
struct TempTree(PathBuf);

impl TempTree {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "providock-scan-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn copy_file(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
    std::fs::copy(src, dst).unwrap_or_else(|e| panic!("copy {src:?} -> {dst:?}: {e}"));
}

fn write_file(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn append_line(path: &Path, line: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{line}").unwrap();
}

fn open_db() -> Database {
    Database::open_in_memory().unwrap()
}

fn src_of(
    agent: AgentKind,
    source_id: &str,
    root: &Path,
    parser_version: &'static str,
) -> DiscoveredSource {
    DiscoveredSource {
        source_id: source_id.into(),
        agent,
        root: root.to_path_buf(),
        parser_version,
        resolution: "test".into(),
    }
}

fn count(db: &Database) -> i64 {
    db.conn()
        .query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))
        .unwrap()
}

fn sum(db: &Database, col: &str) -> Option<i64> {
    db.conn()
        .query_row(&format!("SELECT SUM({col}) FROM usage_events"), [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .unwrap()
}

fn assert_sum(db: &Database, col: &str, want: i64) {
    assert_eq!(sum(db, col), Some(want), "SUM({col})");
}

// ---------------------------------------------------------------------------
// A01 Claude Code
// ---------------------------------------------------------------------------

#[test]
fn claude_code_scan_totals_append_and_idempotent() {
    let tree = TempTree::new("claude");
    let session = tree.path().join("projects/proj-a/sess-A.jsonl");
    copy_file(
        &fixtures().join("claude_code/projects-jsonl/multi-message/input/session.jsonl"),
        &session,
    );
    let src = src_of(
        AgentKind::ClaudeCode,
        "claude_code:test",
        tree.path(),
        claude_code::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 3, "diag={:?}", r1.diagnostics);
    assert_eq!(count(&db), 3);
    // 夹具 expected.json：195+1800+300 = 2295；2295+73 = 2368
    assert_sum(&db, "input_fresh", 195);
    assert_sum(&db, "input_cache_read", 1800);
    assert_sum(&db, "input_cache_write", 300);
    assert_sum(&db, "input_total", 2295);
    assert_sum(&db, "output_total", 73);
    assert_sum(&db, "total", 2368);

    // 重扫幂等（tail_checksum 未变 → 整文件跳过）。
    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(r2.files_unchanged, 1);
    assert_eq!(count(&db), 3);

    // 追加 msg_004：in 10 out 5 read 0 write 0 → input_total 10、total 15。
    append_line(
        &session,
        r#"{"type":"assistant","sessionId":"sess-A","timestamp":"2026-10-11T01:01:00.000Z","message":{"id":"msg_004","model":"claude-sonnet-4-20250514","stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
    );
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 2383); // 2368 + 15
    assert_sum(&db, "input_total", 2305); // 2295 + 10

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 2383);
}

#[test]
fn claude_code_subagent_actor_from_relative_path() {
    let tree = TempTree::new("claude-sub");
    copy_file(
        &fixtures().join("claude_code/projects-jsonl/subagent/input/agent-7f3a.jsonl"),
        &tree
            .path()
            .join("projects/proj-a/sess-parent/subagents/agent-7f3a.jsonl"),
    );
    let src = src_of(
        AgentKind::ClaudeCode,
        "claude_code:test",
        tree.path(),
        claude_code::PARSER_VERSION,
    );
    let mut db = open_db();

    let r = scan_source(&mut db, &src).unwrap();
    assert_eq!(r.events_inserted, 1, "diag={:?}", r.diagnostics);
    assert_sum(&db, "total", 460);

    let (session, parent, actor): (Option<String>, Option<String>, Option<String>) = db
        .conn()
        .query_row(
            "SELECT session_id, parent_session_id, actor_id FROM usage_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    // actor 取自相对会话目录的路径段（去掉 .jsonl）：subagents/agent-7f3a
    assert_eq!(actor.as_deref(), Some("subagents/agent-7f3a"));
    assert_eq!(session.as_deref(), Some("sess-parent"));
    assert_eq!(parent.as_deref(), Some("sess-parent"));

    let has_subagents: i64 = db
        .conn()
        .query_row(
            "SELECT has_subagents FROM sessions WHERE session_id = 'sess-parent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_subagents, 1);
}

// ---------------------------------------------------------------------------
// A03 Gemini CLI
// ---------------------------------------------------------------------------

#[test]
fn gemini_json_scan_totals_and_rewrite() {
    let tree = TempTree::new("gemini-json");
    let session = tree.path().join("tmp/deadbeef/chats/session-full-01.json");
    copy_file(
        &fixtures().join("gemini_cli/chats/full-json-multi/input/session-full-01.json"),
        &session,
    );
    let src = src_of(
        AgentKind::GeminiCli,
        "gemini_cli:test",
        tree.path(),
        gemini_cli::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 3, "diag={:?}", r1.diagnostics);
    // 夹具 totals：1300+400 = 1700；150+1700 = 1850（thoughts 60 计入输出）
    assert_sum(&db, "input_fresh", 1300);
    assert_sum(&db, "input_cache_read", 400);
    assert_sum(&db, "input_total", 1700);
    assert_sum(&db, "output_total", 150);
    assert_sum(&db, "output_reasoning", 60);
    assert_sum(&db, "total", 1850);

    // JSON 形态 mtime+size 短路：文件未变直接跳过。
    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(r2.files_unchanged, 1);
    assert_eq!(count(&db), 3);

    // 改写整个 JSON（模拟会话文件快照更新）：追加 m5 in 8 out 2 → total +10。
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&session).unwrap()).unwrap();
    doc["messages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "m5",
            "type": "gemini",
            "timestamp": "2026-10-11T01:01:00.000Z",
            "model": "gemini-2.5-flash",
            "tokens": { "input": 8, "output": 2, "thoughts": 0, "cached": 0 }
        }));
    std::fs::write(&session, serde_json::to_vec(&doc).unwrap()).unwrap();

    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 1860); // 1850 + 10

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 4);
}

#[test]
fn gemini_jsonl_prefers_jsonl_and_appends() {
    let tree = TempTree::new("gemini-jsonl");
    let chats = tree.path().join("tmp/hash-1/chats");
    let session = chats.join("session-set-rewind-02.jsonl");
    copy_file(
        &fixtures().join("gemini_cli/chats/jsonl-set-rewind/input/session-set-rewind-02.jsonl"),
        &session,
    );
    // 同名 .json 干扰：若被误读会导入 99999 input，断言不会被读到。
    write_file(
        &chats.join("session-set-rewind-02.json"),
        r#"{"sessionId":"decoy","messages":[{"id":"x","type":"gemini","tokens":{"input":99999,"output":0,"thoughts":0,"cached":0}}]}"#,
    );
    let src = src_of(
        AgentKind::GeminiCli,
        "gemini_cli:test",
        tree.path(),
        gemini_cli::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 3, "diag={:?}", r1.diagnostics);
    // 夹具 totals：560+160 = 720；88+720 = 808
    assert_sum(&db, "input_fresh", 560);
    assert_sum(&db, "input_cache_read", 160);
    assert_sum(&db, "input_total", 720);
    assert_sum(&db, "output_total", 88);
    assert_sum(&db, "total", 808);

    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(count(&db), 3);

    // JSONL 追加 m5 in 100 out 10 → input_total 100、total 110。
    append_line(
        &session,
        r#"{"id":"m5","type":"gemini","timestamp":"2026-10-11T02:02:00.000Z","model":"gemini-2.5-pro","tokens":{"input":100,"output":10,"thoughts":0,"cached":0}}"#,
    );
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 918); // 808 + 110

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 4);
}

// ---------------------------------------------------------------------------
// A04 Grok Build
// ---------------------------------------------------------------------------

#[test]
fn grok_scan_totals_append_and_idempotent() {
    let tree = TempTree::new("grok");
    let updates = tree
        .path()
        .join("sessions/enc-cwd/0199aaaa-0000-7000-8000-000000000001/updates.jsonl");
    copy_file(
        &fixtures().join("grok_build/updates-jsonl/two-turns-identical/input/updates.jsonl"),
        &updates,
    );
    let src = src_of(
        AgentKind::GrokBuild,
        "grok_build:test",
        tree.path(),
        grok_build::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 2, "diag={:?}", r1.diagnostics);
    // 每轮：fresh 1000 + read 400 = input_total 1400，output 200 → total 1600；两轮 3200
    assert_sum(&db, "input_fresh", 2000);
    assert_sum(&db, "input_cache_read", 800);
    assert_sum(&db, "input_total", 2800);
    assert_sum(&db, "output_total", 400);
    assert_sum(&db, "output_reasoning", 100);
    assert_sum(&db, "total", 3200);
    assert_sum(&db, "request_count", 4); // modelCalls 2 × 2 轮

    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(count(&db), 2);

    // 追加第三轮（数字相同也必须各计一次）：total +1600、request_count +2。
    append_line(
        &updates,
        r#"{"jsonrpc":"2.0","method":"_x.ai/session/update","params":{"sessionId":"0199aaaa-0000-7000-8000-000000000001","update":{"sessionUpdate":"turn_completed","prompt_id":"0199bbbb-0000-7000-8000-000000000003","usage":{"inputTokens":1000,"outputTokens":200,"cachedReadTokens":400,"reasoningTokens":50,"modelCalls":2}}},"timestamp":1791626520}"#,
    );
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 3);
    assert_sum(&db, "total", 4800); // 3200 + 1600
    assert_sum(&db, "request_count", 6);

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 3);
}

#[test]
fn grok_archived_sessions_marked_archived() {
    let tree = TempTree::new("grok-arch");
    copy_file(
        &fixtures().join("grok_build/updates-jsonl/two-turns-identical/input/updates.jsonl"),
        &tree
            .path()
            .join("archived_sessions/enc/sess-1/updates.jsonl"),
    );
    let src = src_of(
        AgentKind::GrokBuild,
        "grok_build:test",
        tree.path(),
        grok_build::PARSER_VERSION,
    );
    let mut db = open_db();

    let r = scan_source(&mut db, &src).unwrap();
    assert_eq!(r.events_inserted, 2, "diag={:?}", r.diagnostics);
    let archived: i64 = db
        .conn()
        .query_row(
            "SELECT archived FROM sessions WHERE session_id = 'sess-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1);
}

// ---------------------------------------------------------------------------
// A05 OpenCode（只读 SQLite，mtime 游标）
// ---------------------------------------------------------------------------

fn opencode_message_data(input: u64, output: u64) -> serde_json::Value {
    serde_json::json!({
        "model": { "id": "claude-opus-4" },
        "cost": 0.0,
        "tokens": {
            "input": input,
            "output": output,
            "reasoning": 0,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1760001060000i64, "completed": 1760001061000i64 }
    })
}

#[test]
fn opencode_scan_totals_idempotent_and_update_correction() {
    let tree = TempTree::new("opencode");
    let db_path = tree.path().join("opencode.db");
    copy_file(
        &fixtures().join("opencode/sqlite-v2/input/opencode.db"),
        &db_path,
    );
    let src = src_of(
        AgentKind::Opencode,
        "opencode:test",
        &db_path,
        opencode_db::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 4, "diag={:?}", r1.diagnostics);
    // 夹具 totals：fresh 500+0+300+100 = 900；read 110；write 25；input_total 1035；
    // output 50+0+50+20 = 120；total 1155
    assert_sum(&db, "input_fresh", 900);
    assert_sum(&db, "input_cache_read", 110);
    assert_sum(&db, "input_cache_write", 25);
    assert_sum(&db, "input_total", 1035);
    assert_sum(&db, "output_total", 120);
    assert_sum(&db, "total", 1155);

    // db（含 -wal）mtime+size 未变 → 整库跳过。
    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(r2.files_unchanged, 1);
    assert_eq!(count(&db), 4);

    // 上游更新 a2 行：input 0→1000、output 0→500。重扫应校正而非双算。
    {
        let conn = Connection::open(&db_path).unwrap();
        let data = serde_json::to_string(&opencode_message_data(1000, 500)).unwrap();
        conn.execute(
            "UPDATE session_message SET data = ?1 WHERE id = 'a2'",
            rusqlite::params![data],
        )
        .unwrap();
    }
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 0, "diag={:?}", r3.diagnostics);
    // OpenCode 解析器对每条事件都标 replace_existing：整库重读 = 全部按 id 覆盖。
    assert_eq!(r3.events_replaced, 4);
    assert_eq!(count(&db), 4, "更新已存在的 message id 不新增事件");
    // 校正后：fresh 900+1000 = 1900；input_total 1035-0+1000 = 2035；total 1155+1500-0 = 2655
    assert_sum(&db, "input_fresh", 1900);
    assert_sum(&db, "input_total", 2035);
    assert_sum(&db, "total", 2655);

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 2655);
}

// ---------------------------------------------------------------------------
// A06 Pi
// ---------------------------------------------------------------------------

#[test]
fn pi_scan_totals_append_and_idempotent() {
    let tree = TempTree::new("pi");
    let session_dir = tree.path().join("sessions");
    let session = session_dir.join("2026-10-11T01-00-00-000Z_3f2a9c1d.jsonl");
    copy_file(
        &fixtures().join(
            "pi/sessions-jsonl/assistant-usage/input/2026-10-11T01-00-00-000Z_3f2a9c1d.jsonl",
        ),
        &session,
    );
    let src = src_of(
        AgentKind::Pi,
        "pi:test",
        &session_dir,
        pi_log::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 2, "diag={:?}", r1.diagnostics);
    // 夹具 totals：300+30+5 = 335；335+90 = 425
    assert_sum(&db, "input_fresh", 300);
    assert_sum(&db, "input_cache_read", 30);
    assert_sum(&db, "input_cache_write", 5);
    assert_sum(&db, "input_total", 335);
    assert_sum(&db, "output_total", 90);
    assert_sum(&db, "total", 425);

    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(count(&db), 2);

    // 追加一条 assistant message：in 100 out 20 → input_total 100、total 120。
    append_line(
        &session,
        r#"{"type":"message","id":"00000005","parentId":"00000004","timestamp":"2026-10-11T01:00:05.000Z","message":{"role":"assistant","model":"claude-sonnet-4-5","usage":{"input":100,"output":20,"cacheRead":0,"cacheWrite":0},"stopReason":"stop"}}"#,
    );
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 3);
    assert_sum(&db, "total", 545); // 425 + 120

    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 3);
}

// ---------------------------------------------------------------------------
// A07 MCode（只读 SQLite，row_id 游标）
// ---------------------------------------------------------------------------

#[test]
fn mcode_scan_row_cursor_totals_and_incremental() {
    let tree = TempTree::new("mcode");
    let db_path = tree.path().join("v2/sqlite/runtime-state.sqlite");
    copy_file(
        &fixtures().join("mcode/runtime-state-sqlite/basic-multi-row/runtime-state.sqlite"),
        &db_path,
    );
    let src = src_of(
        AgentKind::Mcode,
        "mcode:test",
        tree.path(),
        mcode_db::PARSER_VERSION,
    );
    let mut db = open_db();

    let r1 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r1.events_inserted, 3, "diag={:?}", r1.diagnostics);
    // 夹具 totals：fresh 1000+500+0 = 1500；read 300；write 100；input_total 1900；
    // output 280；reasoning 50；total 2180
    assert_sum(&db, "input_fresh", 1500);
    assert_sum(&db, "input_cache_read", 300);
    assert_sum(&db, "input_cache_write", 100);
    assert_sum(&db, "input_total", 1900);
    assert_sum(&db, "output_total", 280);
    assert_sum(&db, "output_reasoning", 50);
    assert_sum(&db, "total", 2180);

    // 无新行：重叠回看命中已存在 id，覆盖校正而不新增（游标推进不重复计数）。
    let r2 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r2.events_inserted, 0);
    assert_eq!(count(&db), 3);
    assert_sum(&db, "total", 2180);

    // 追加 id=4：in 10 out 5 → input_total 10、total 15。
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO local_runtime_token_usage
             (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
              cache_read_tokens, cache_write_tokens, cost_usd)
             VALUES (4, 'sess-alpha', 'MiniMax-M2', 1791705603000, 10, 5, 0, 0, 0, NULL)",
            [],
        )
        .unwrap();
    }
    let r3 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r3.events_inserted, 1, "diag={:?}", r3.diagnostics);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 2195); // 2180 + 15

    // id 游标已推进到 4：再次重扫不重复。
    let r4 = scan_source(&mut db, &src).unwrap();
    assert_eq!(r4.events_inserted, 0);
    assert_eq!(count(&db), 4);
    assert_sum(&db, "total", 2195);
}

use providock_lib::{
    adapters::agents::deepseek_harness,
    domain::AgentKind,
    ingestion::scanner::{scan_source, DiscoveredSource},
    storage::Database,
};
use std::path::PathBuf;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "providock-dsh-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn put(&self, name: &str, body: &[u8]) {
        let p = self.0.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    fn source(&self) -> DiscoveredSource {
        DiscoveredSource {
            source_id: "dsh-fixture".into(),
            agent: AgentKind::DeepseekHarness,
            root: self.0.clone(),
            parser_version: deepseek_harness::PARSER_VERSION,
            resolution: "synthetic".into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const DATA: &[u8] = include_bytes!("../../fixtures/agents/deepseek_harness/session-v4/input.jsonl");
fn total(db: &Database) -> i64 {
    db.conn()
        .query_row("SELECT COALESCE(SUM(total),0) FROM usage_events", [], |r| {
            r.get(0)
        })
        .unwrap()
}
#[test]
fn compressed_generations_and_copies_are_idempotent_and_unknown_versions_keep_history() {
    let fixture = Fixture::new();
    let older = String::from_utf8(DATA.to_vec())
        .unwrap()
        .replace("\"version\":4", "\"version\":3");
    fixture.put("project/session/session.v3.jsonl", older.as_bytes());
    fixture.put(
        "project/session/session.v4.jsonl.zstd",
        &zstd::stream::encode_all(DATA, 1).unwrap(),
    );
    let mut db = Database::open_in_memory().unwrap();
    let first = scan_source(&mut db, &fixture.source()).unwrap();
    assert_eq!(first.files_seen, 1);
    assert_eq!(first.events_inserted, 4);
    assert_eq!(total(&db), 612);
    assert_eq!(
        scan_source(&mut db, &fixture.source())
            .unwrap()
            .events_inserted,
        0
    );
    assert_eq!(total(&db), 612);
    fixture.put("project/copy/session.v4.jsonl", DATA);
    assert_eq!(
        scan_source(&mut db, &fixture.source())
            .unwrap()
            .events_inserted,
        0
    );
    assert_eq!(total(&db), 612);
    fixture.put(
        "project/session/session.v5.jsonl",
        String::from_utf8(DATA.to_vec())
            .unwrap()
            .replace("\"version\":4", "\"version\":5")
            .as_bytes(),
    );
    assert_eq!(
        scan_source(&mut db, &fixture.source())
            .unwrap()
            .files_deferred,
        1
    );
    assert_eq!(total(&db), 612);
}
#[test]
fn fork_skips_inherited_prefix_and_orphan_defers_without_cursor() {
    let fixture = Fixture::new();
    let mut rows: Vec<serde_json::Value> = String::from_utf8(DATA.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    rows[0]["id"] = "synthetic-child".into();
    rows[0]["parentSession"] = "synthetic-dsh".into();
    rows[0]["isSeeded"] = true.into();
    rows.push(serde_json::json!({"type":"session/end-seed","seq":6,"time":1791849606000i64,"data":{"inherited":true}}));
    rows.push(serde_json::json!({"type":"assistant/message","seq":7,"time":1791849607000i64,"data":{"turn":1,"step":0,"message":{"source":{"model":"deepseek-flash"}},"usage":{"inputTokens":20,"outputTokens":5,"cacheReadTokens":0,"cacheWriteTokens":0,"totalTokens":25}}}));
    let child = rows.iter().map(|v| format!("{v}\n")).collect::<String>();
    fixture.put("project/child/session.v4.jsonl", child.as_bytes());
    let mut db = Database::open_in_memory().unwrap();
    let orphan = scan_source(&mut db, &fixture.source()).unwrap();
    assert_eq!(orphan.files_deferred, 1);
    assert_eq!(total(&db), 0);
    let cursors: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM ingestion_cursors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cursors, 0);
    fixture.put("project/parent/session.v4.jsonl", DATA);
    let result = scan_source(&mut db, &fixture.source()).unwrap();
    assert_eq!(result.events_inserted, 5);
    assert_eq!(total(&db), 637);
    assert_eq!(
        scan_source(&mut db, &fixture.source())
            .unwrap()
            .events_inserted,
        0
    );
    assert_eq!(total(&db), 637);
}

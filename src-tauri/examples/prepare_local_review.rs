//! Creates a fresh, synthetic review database at an explicit non-production path.
use providock_lib::{
    adapters::agents::deepseek_harness,
    domain::AgentKind,
    ingestion::scanner::{scan_source, DiscoveredSource},
    storage::Database,
};
fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("explicit empty review directory required");
    let root = std::path::PathBuf::from(root);
    assert!(
        !root.join("providock.db").exists(),
        "review database must be new"
    );
    std::fs::create_dir_all(&root).unwrap();
    let sessions = root.join("synthetic-sessions/project/session");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("session.v4.jsonl"),
        include_bytes!("../../fixtures/agents/deepseek_harness/session-v4/input.jsonl"),
    )
    .unwrap();
    let mut db = Database::open(&root.join("providock.db")).unwrap();
    for key in ["usage.auto_sync_enabled", "accounts.scheduler.enabled"] {
        db.conn()
            .execute("INSERT INTO settings(key,value) VALUES (?1,'false')", [key])
            .unwrap();
    }
    let report = scan_source(
        &mut db,
        &DiscoveredSource {
            source_id: "synthetic-review-dsh".into(),
            agent: AgentKind::DeepseekHarness,
            root: root.join("synthetic-sessions"),
            parser_version: deepseek_harness::PARSER_VERSION,
            resolution: "synthetic review fixture".into(),
        },
    )
    .unwrap();
    assert_eq!(report.events_inserted, 4);
    println!("Synthetic review database prepared: 4 calls, 612 tokens; automatic background queries disabled.");
}

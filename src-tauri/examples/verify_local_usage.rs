//! 只读实机核对工具：只扫描 Kimi/Codex，使用内存数据库，不读凭据、不启动推理。
use providock_lib::{
    domain::AgentKind,
    ingestion::scanner::{discover_sources, scan_source},
    services::aggregate::{usage_summary, UsageFilter},
    storage::Database,
};
fn main() {
    let mut db = Database::open_in_memory().expect("memory database");
    let sources = discover_sources()
        .into_iter()
        .filter(|source| matches!(source.agent, AgentKind::KimiCode | AgentKind::Codex))
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    for source in sources {
        let first = scan_source(&mut db, &source).expect("first scan");
        let filter = UsageFilter {
            agent: Some(source.agent.as_str().into()),
            ..Default::default()
        };
        let before = usage_summary(&db, &filter).unwrap();
        let second = scan_source(&mut db, &source).expect("repeat scan");
        let after = usage_summary(&db, &filter).unwrap();
        let today = usage_summary(
            &db,
            &UsageFilter {
                from: std::env::var("PROVIDOCK_VERIFY_FROM").ok(),
                to: std::env::var("PROVIDOCK_VERIFY_TO").ok(),
                agent: filter.agent,
                ..Default::default()
            },
        )
        .unwrap();
        result.push(serde_json::json!({"agent":source.agent.as_str(),"files":first.files_seen,"deferred":first.files_deferred,"diagnostics":first.diagnostics.len(),"summary":before,"after":after,"today":today,"repeatInserted":second.events_inserted,"repeatReplaced":second.events_replaced}));
    }
    println!("{}", serde_json::to_string(&result).unwrap());
}

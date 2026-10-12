//! 用量页可见 Agent 列表过滤：三条查询路径（summary / dashboard / events）必须同时受限，
//! `agents: None` 保持既有语义（不限制），空列表匹配不到任何行。
use providock_lib::services::{
    aggregate::{usage_series_daily, usage_summary, UsageFilter},
    usage_details,
};
use providock_lib::storage::Database;

fn db() -> Database {
    let db = Database::open_in_memory().unwrap();
    for (source, agent) in [("s1", "codex"), ("s2", "kimi_code")] {
        db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES (?1,?2,'local_log','/synthetic','test')",rusqlite::params![source,agent]).unwrap();
    }
    db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s1','test','committed')",[]).unwrap();
    db
}
fn event(db: &Database, id: &str, source: &str, agent: &str, model: &str, total: i64) {
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,model_normalized,input_fresh,input_cache_read,input_cache_write,input_total,output_total,total,tokens_quality) VALUES (?1,1,1,?2,?3,'local_log','2026-10-12T10:00:00Z','2026-10-12T10:00:00Z','request',?4,1000,500,0,1500,200,?5,'reported')",rusqlite::params![id,source,agent,model,total]).unwrap();
}
/// 合成账本：codex 两笔（1700+111），kimi_code 一笔（300），同一天。
fn seeded() -> Database {
    let db = db();
    event(&db, "e1", "s1", "codex", "m1", 1700);
    event(&db, "e2", "s2", "kimi_code", "m2", 300);
    event(&db, "e3", "s1", "codex", "m1", 111);
    db
}

#[test]
fn agents_list_restricts_summary_dashboard_and_events() {
    let db = seeded();
    let filter = UsageFilter {
        agents: Some(vec!["kimi_code".into()]),
        ..Default::default()
    };
    let summary = usage_summary(&db, &filter).unwrap();
    assert_eq!(summary.events, 1);
    assert_eq!(summary.total.value.as_deref(), Some("300"));
    assert_eq!(
        summary
            .by_agent
            .iter()
            .map(|b| b.agent.as_str())
            .collect::<Vec<_>>(),
        ["kimi_code"]
    );
    let dashboard = usage_details::dashboard(&db, &filter, true).unwrap();
    assert_eq!(
        dashboard
            .agents
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["kimi_code"]
    );
    assert_eq!(dashboard.models.len(), 1);
    assert_eq!(dashboard.models[0].name, "m2");
    let page = usage_details::events(&db, &filter, 1, 10).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].agent, "kimi_code");
    // 同一过滤也约束趋势序列（与 summary 同口径）。
    let series = usage_series_daily(&db, &filter).unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].1.as_deref(), Some("300"));
    // 多个 ID 的列表：两行都在。
    let both = UsageFilter {
        agents: Some(vec!["kimi_code".into(), "codex".into()]),
        ..Default::default()
    };
    assert_eq!(usage_summary(&db, &both).unwrap().events, 3);
    assert_eq!(usage_details::events(&db, &both, 1, 10).unwrap().total, 3);
}

#[test]
fn agents_none_matches_legacy_semantics_exactly() {
    let db = seeded();
    let default = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(default.events, 3);
    assert_eq!(default.total.value.as_deref(), Some("2111"));
    let explicit_none = usage_summary(
        &db,
        &UsageFilter {
            agents: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(default.events, explicit_none.events);
    assert_eq!(default.total.value, explicit_none.total.value);
    // 未传 agents 的 dashboard / events 与既有行为一致（3 行全部可见）。
    let dashboard = usage_details::dashboard(&db, &UsageFilter::default(), true).unwrap();
    assert_eq!(dashboard.agents.len(), 2);
    assert_eq!(
        usage_details::events(&db, &UsageFilter::default(), 1, 10)
            .unwrap()
            .total,
        3
    );
    // 显式传入全部白名单与 None 等价。
    let all: Vec<String> = [
        "claude_code",
        "codex",
        "gemini_cli",
        "grok_build",
        "opencode",
        "pi",
        "mcode",
        "kimi_code",
    ]
    .iter()
    .map(|id| (*id).to_owned())
    .collect();
    let full = UsageFilter {
        agents: Some(all),
        ..Default::default()
    };
    assert_eq!(usage_summary(&db, &full).unwrap().events, 3);
    assert_eq!(usage_details::events(&db, &full, 1, 10).unwrap().total, 3);
}

#[test]
fn empty_agents_list_matches_nothing_in_every_query_path() {
    let db = seeded();
    let filter = UsageFilter {
        agents: Some(vec![]),
        ..Default::default()
    };
    let summary = usage_summary(&db, &filter).unwrap();
    assert_eq!(summary.events, 0);
    assert!(summary.by_agent.is_empty());
    assert!(summary.total.value.is_none());
    let dashboard = usage_details::dashboard(&db, &filter, true).unwrap();
    assert!(dashboard.agents.is_empty());
    assert!(dashboard.models.is_empty());
    assert!(dashboard.series.is_empty());
    let page = usage_details::events(&db, &filter, 1, 10).unwrap();
    assert_eq!(page.total, 0);
    assert!(page.rows.is_empty());
    assert!(usage_series_daily(&db, &filter).unwrap().is_empty());
}

#[test]
fn agents_list_intersects_with_single_agent_predicate() {
    let db = seeded();
    let filter = UsageFilter {
        agent: Some("codex".into()),
        agents: Some(vec!["kimi_code".into()]),
        ..Default::default()
    };
    assert_eq!(usage_summary(&db, &filter).unwrap().events, 0);
    assert_eq!(usage_details::events(&db, &filter, 1, 10).unwrap().total, 0);
    let filter = UsageFilter {
        agent: Some("codex".into()),
        agents: Some(vec!["kimi_code".into(), "codex".into()]),
        ..Default::default()
    };
    let summary = usage_summary(&db, &filter).unwrap();
    assert_eq!(summary.events, 2);
    assert_eq!(summary.total.value.as_deref(), Some("1811"));
}

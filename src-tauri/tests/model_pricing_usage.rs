use providock_lib::{
    domain::{Completeness, Quality, TokenSet},
    services::{
        aggregate::{self, UsageFilter},
        model_prices::{self, ModelPrice},
        pricing, usage_details,
    },
    storage::Database,
};
fn db() -> Database {
    let db = Database::open_in_memory().unwrap();
    for (source, agent) in [("s1", "codex"), ("s2", "kimi_code"), ("s3", "codex")] {
        db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES (?1,?2,'local_log','/synthetic','test')",rusqlite::params![source,agent]).unwrap();
    }
    db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s1','test','committed')",[]).unwrap();
    db
}
fn event(db: &Database, id: &str, source: &str, agent: &str, model: Option<&str>, at: &str) {
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,model_raw,input_fresh,input_cache_read,input_cache_write,input_total,output_total,total,tokens_quality,cost_reported_amount,cost_reported_currency) VALUES (?1,1,1,?2,?3,'local_log',?5,?5,'request',?4,1000,500,0,1500,200,1700,'reported','0','USD')",rusqlite::params![id,source,agent,model,at]).unwrap();
}
fn rule(agent: &str, model: &str) -> ModelPrice {
    ModelPrice {
        source: None,
        version: String::new(),
        agent: agent.into(),
        source_id: None,
        model: model.into(),
        currency: "USD".into(),
        input: Some("3".into()),
        output: Some("12".into()),
        cache_read: Some("0.3".into()),
        cache_write: None,
        enabled: true,
        peak: None,
        peak_windows: None,
        long_context: None,
    }
}
fn tokens() -> TokenSet {
    TokenSet {
        input_fresh: Some(1000),
        input_cache_read: Some(500),
        input_cache_write: Some(0),
        input_total: Some(1500),
        output_total: Some(200),
        output_reasoning: None,
        total: Some(1700),
        quality: Quality::Reported,
    }
}
#[test]
fn per_million_cost_is_exact_and_cache_not_double_counted() {
    let cost = model_prices::quote(&rule("codex", "m1"), &tokens(), None, None)
        .unwrap()
        .unwrap();
    // 1000*3/1M + 500*0.3/1M + 200*12/1M = 0.00555 (not total input * 3).
    assert_eq!(cost.money.amount, "0.00555");
    assert_eq!(cost.completeness, Completeness::Complete);
    let mut p = rule("codex", "m1");
    p.cache_read = None;
    assert_eq!(
        model_prices::quote(&p, &tokens(), None, None)
            .unwrap()
            .unwrap()
            .completeness,
        Completeness::Partial
    );
    let mut t = tokens();
    t.input_cache_write = None;
    assert_eq!(
        model_prices::quote(&p, &t, None, None)
            .unwrap()
            .unwrap()
            .completeness,
        Completeness::Partial
    );
}
#[test]
fn invalid_prices_are_rejected_unknown_and_zero_are_distinct() {
    let db = db();
    for value in ["-1", "NaN", "1e3", "1.", "9999999999", "0.1234567891"] {
        let mut p = rule("codex", "m1");
        p.input = Some(value.into());
        assert!(model_prices::save(&db, p).is_err(), "{value}");
    }
    let mut p = rule("codex", "m1");
    p.input = None;
    p.output = None;
    p.cache_read = None;
    assert!(model_prices::save(&db, p.clone()).is_err());
    assert!(model_prices::quote(&p, &tokens(), None, None)
        .unwrap()
        .is_none());
    p.input = Some("0".into());
    p.output = Some("0".into());
    p.cache_read = Some("0".into());
    assert_eq!(
        model_prices::quote(&p, &tokens(), None, None)
            .unwrap()
            .unwrap()
            .money
            .amount,
        "0"
    );
}
#[test]
fn price_versions_keep_history_and_source_overrides_do_not_cross_agents() {
    let db = db();
    let p = model_prices::save(&db, rule("codex", "m1")).unwrap();
    let mut source = p.clone();
    source.source_id = Some("s3".into());
    source.input = Some("10".into());
    let source = model_prices::save(&db, source).unwrap();
    let rules = model_prices::list(&db).unwrap();
    assert_eq!(
        model_prices::matching(&rules, "codex", "s3", Some("m1"))
            .unwrap()
            .input
            .as_deref(),
        Some("10")
    );
    assert_eq!(
        model_prices::matching(&rules, "codex", "s1", Some("m1"))
            .unwrap()
            .input
            .as_deref(),
        Some("3")
    );
    assert!(model_prices::matching(&rules, "kimi_code", "s2", Some("m1")).is_none());
    assert!(model_prices::matching(&rules, "codex", "s1", None).is_none());
    let mut updated = p.clone();
    updated.input = Some("4".into());
    model_prices::save(&db, updated).unwrap();
    let mut disabled = source;
    disabled.enabled = false;
    model_prices::save(&db, disabled).unwrap();
    assert_eq!(model_prices::list(&db).unwrap().len(), 1);
    assert_eq!(pricing::price_versions_list(&db).unwrap().len(), 4);
    assert_eq!(
        pricing::price_versions_list(&db)
            .unwrap()
            .iter()
            .find(|v| v.price_version == p.version)
            .unwrap()
            .rates
            .input
            .as_deref(),
        Some("3")
    );
}
#[test]
fn model_groups_filters_event_pagination_and_multi_currency_agree() {
    let db = db();
    for (id, source, agent, model) in [
        ("e1", "s1", "codex", "m1"),
        ("e2", "s1", "codex", "m2"),
        ("e3", "s2", "kimi_code", "m1"),
        ("e4", "s1", "codex", "m1"),
    ] {
        event(&db, id, source, agent, Some(model), "2026-10-12T10:00:00Z");
    }
    model_prices::save(&db, rule("codex", "m1")).unwrap();
    let mut cny = rule("kimi_code", "m1");
    cny.currency = "CNY".into();
    model_prices::save(&db, cny).unwrap();
    let all = UsageFilter::default();
    let d = usage_details::dashboard(&db, &all, true).unwrap();
    assert_eq!(d.models.len(), 3);
    assert_eq!(d.models[0].tokens.total.as_deref(), Some("3400"));
    assert_eq!(d.priced_events, 3);
    assert_eq!(d.current_cost.len(), 2);
    assert_eq!(
        d.current_cost
            .iter()
            .find(|a| a.currency == "USD")
            .unwrap()
            .amount,
        "0.0111"
    );
    assert_eq!(d.series[0].tokens.total.as_deref(), Some("6800"));
    let filter = UsageFilter {
        agent: Some("codex".into()),
        model: Some("m1".into()),
        from: Some("2026-10-12T00:00:00Z".into()),
        to: Some("2026-10-13T00:00:00Z".into()),
        ..Default::default()
    };
    let d = usage_details::dashboard(&db, &filter, true).unwrap();
    let summary = aggregate::usage_summary(&db, &filter).unwrap();
    assert!(serde_json::to_value(&summary)
        .unwrap()
        .get("costReported")
        .is_none());
    let legacy: String = db
        .conn()
        .query_row(
            "SELECT cost_reported_amount FROM usage_events WHERE event_id='e1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        legacy, "0",
        "legacy column remains untouched for upgrade compatibility"
    );
    assert_eq!(d.models.len(), 1);
    assert_eq!(d.models[0].tokens.total, summary.total.value);
    assert_eq!(d.priced_events, 2);
    let first = usage_details::events(&db, &filter, 1, 1).unwrap();
    let second = usage_details::events(&db, &filter, 2, 1).unwrap();
    assert_eq!(first.total, 2);
    assert_ne!(first.rows[0].event_id, second.rows[0].event_id);
    assert!(serde_json::to_value(&first.rows[0])
        .unwrap()
        .get("costReported")
        .is_none());
    assert_eq!(
        first.rows[0].cost_calculated.as_ref().unwrap().amount,
        "0.00555"
    );
    let stored: Option<String> = db
        .conn()
        .query_row(
            "SELECT cost_estimated_amount FROM usage_events WHERE event_id='e1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        stored.is_none(),
        "display comparison must not rewrite historical estimates"
    );
}
#[test]
fn unknowns_domains_offsets_and_bounds_are_preserved() {
    let db = db();
    event(
        &db,
        "unknown",
        "s1",
        "codex",
        None,
        "2026-10-12T01:00:00+08:00",
    );
    db.conn().execute("UPDATE usage_events SET total=NULL,input_fresh=NULL,input_cache_read=NULL,input_cache_write=NULL,input_total=NULL,output_total=NULL WHERE event_id='unknown'",[]).unwrap();
    event(
        &db,
        "gateway",
        "s1",
        "codex",
        Some("m1"),
        "2026-10-12T12:00:00Z",
    );
    db.conn()
        .execute(
            "UPDATE usage_events SET stream_domain='gateway' WHERE event_id='gateway'",
            [],
        )
        .unwrap();
    let d = usage_details::dashboard(&db, &UsageFilter::default(), true).unwrap();
    assert!(d.models[0].tokens.total.is_none());
    assert_eq!(d.series[0].at, "2026-10-11T17:00:00Z");
    assert!(d.current_cost.is_empty());
    let page = usage_details::events(&db, &UsageFilter::default(), u32::MAX, u32::MAX).unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.page_size, 100);
    assert_eq!(page.total, 1);
}

use providock_lib::{
    domain::{Quality, TokenSet},
    services::{
        model_prices::{self, ModelPrice},
        price_page, price_tables,
    },
    storage::Database,
};
fn quote(
    page: &str,
    model: &str,
    currency: &str,
    batch: bool,
) -> providock_lib::services::price_import::PriceQuote {
    let html = match page {
        "mimo" => include_str!("../../fixtures/pricing-pages/mimo.html"),
        "deepseek" => include_str!("../../fixtures/pricing-pages/deepseek.html"),
        _ => include_str!("../../fixtures/pricing-pages/bigmodel.html"),
    };
    price_tables::parse(
        &price_page::visible_text(html),
        "codex",
        model,
        "https://official.example",
        Some(currency),
        batch,
    )
    .unwrap_or_else(|e| {
        panic!(
            "{page}/{model}/{currency}/batch={batch}: {e}\n{}",
            price_page::visible_text(html)
        )
    })
}
#[test]
fn official_tables_have_independent_expected_prices() {
    let q = quote("mimo", "mimo-v2.6-pro", "CNY", false);
    assert_eq!(q.input.as_deref(), Some("3.00"));
    assert_eq!(q.cache_read.as_deref(), Some("0.025"));
    assert_eq!(q.output.as_deref(), Some("6.00"));
    assert_eq!(q.cache_write.as_deref(), Some("0"));
    assert_eq!(
        quote("mimo", "mimo-v2.6-pro", "USD", true).input.as_deref(),
        Some("0.2175")
    );
    assert_eq!(
        quote("mimo", "mimo-v2.5-pro", "CNY", false)
            .input
            .as_deref(),
        Some("3.00")
    );
    let q = quote("deepseek", "deepseek-flash", "CNY", false);
    assert_eq!(q.input.as_deref(), Some("1"));
    assert_eq!(q.output.as_deref(), Some("4"));
    assert_eq!(q.cache_read.as_deref(), Some("0.02"));
    assert_eq!(q.peak.as_ref().unwrap().input.as_deref(), Some("2"));
    assert!(q.peak_windows.unwrap().exclude_china_holidays);
    assert_eq!(
        quote("bigmodel", "glm-5.3", "CNY", false).input.as_deref(),
        Some("8")
    );
    let q = quote("bigmodel", "glm-5.1", "CNY", false);
    let ts = q.context_tiers.unwrap();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].input_max, Some(32000));
    assert_eq!(ts[1].input_min, Some(32000));
    assert_eq!(ts[1].rates.input.as_deref(), Some("8"));
    let q = quote("bigmodel", "glm-4.7", "CNY", false);
    let ts = q.context_tiers.unwrap();
    assert_eq!(ts.len(), 3);
    assert_eq!(ts[0].output_max, Some(200));
    assert_eq!(ts[1].output_min, Some(200));
    assert_eq!(ts[2].input_max, Some(200000));
    let q = quote("bigmodel", "glm-4.7-flash", "CNY", false);
    assert_eq!(q.input.as_deref(), Some("0"));
    assert_eq!(
        q.cache_write, None,
        "hourly cache storage is not per-token cache creation"
    );
}
fn rule(page: &str, model: &str) -> ModelPrice {
    let q = quote(page, model, "CNY", false);
    let mut v = serde_json::to_value(q).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("method");
    o.remove("notes");
    o.remove("sourceUrl");
    o.insert("sourceId".into(), serde_json::Value::Null);
    serde_json::from_value(v).unwrap()
}
#[test]
fn policy_survives_storage_and_selects_exact_boundaries() {
    let db = Database::open_in_memory().unwrap();
    let r = model_prices::save(&db, rule("bigmodel", "glm-4.7")).unwrap();
    assert_eq!(model_prices::list(&db).unwrap()[0], r);
    let estimate = |input, output| {
        let t = TokenSet {
            input_fresh: Some(input),
            input_total: Some(input),
            input_cache_read: Some(0),
            input_cache_write: Some(0),
            output_total: output,
            quality: Quality::Reported,
            ..Default::default()
        };
        model_prices::quote(&r, &t, None, Some(input)).unwrap()
    };
    assert_eq!(estimate(1000, Some(199)).unwrap().money.amount, "0.003592");
    assert_eq!(estimate(1000, Some(200)).unwrap().money.amount, "0.0058");
    assert_eq!(estimate(32000, Some(200)).unwrap().money.amount, "0.1312");
    assert!(estimate(200000, Some(200)).is_none());
    assert!(estimate(1000, None).is_none());
    let r = rule("deepseek", "deepseek-flash");
    let t = TokenSet {
        input_fresh: Some(1000000),
        input_cache_read: Some(0),
        input_cache_write: Some(0),
        output_total: Some(0),
        ..Default::default()
    };
    assert_eq!(
        model_prices::quote(&r, &t, Some("2026-10-01T02:00:00Z"), None)
            .unwrap()
            .unwrap()
            .money
            .amount,
        "1"
    );
    assert_eq!(
        model_prices::quote(&r, &t, Some("2026-10-14T02:00:00Z"), None)
            .unwrap()
            .unwrap()
            .money
            .amount,
        "2"
    );
    assert!(
        model_prices::quote(&r, &t, Some("2027-10-14T02:00:00Z"), None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn aggregate_counters_do_not_masquerade_as_request_context() {
    use providock_lib::services::{aggregate::UsageFilter, usage_details};
    let db = Database::open_in_memory().unwrap();
    db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES ('s','codex','local_log','/synthetic','test')",[]).unwrap();
    db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s','test','committed')",[]).unwrap();
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,model_raw,input_fresh,input_total,output_total,total,tokens_quality) VALUES ('e',1,1,'s','codex','local_log','2026-10-14T02:00:00Z','2026-10-14T02:00:00Z','turn_aggregate','glm-4.7',1000,1000,200,1200,'reported')",[]).unwrap();
    model_prices::save(&db, rule("bigmodel", "glm-4.7")).unwrap();
    assert!(
        usage_details::dashboard(&db, &UsageFilter::default(), false)
            .unwrap()
            .current_cost
            .is_empty()
    );
    db.conn()
        .execute("UPDATE usage_events SET granularity='request'", [])
        .unwrap();
    assert!(
        !usage_details::dashboard(&db, &UsageFilter::default(), false)
            .unwrap()
            .current_cost
            .is_empty()
    );
}

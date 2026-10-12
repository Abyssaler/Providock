use providock_lib::{
    services::{
        model_prices::{self, ModelPrice},
        price_import::{self, ImportTarget},
    },
    storage::Database,
};

const HTML: &str = include_str!("../../fixtures/pricing/aihot/catalogue.html");
fn target(agent: &str, model: &str) -> Option<ImportTarget> {
    Some(ImportTarget {
        agent: agent.into(),
        model: model.into(),
    })
}
fn manual(model: &str) -> ModelPrice {
    ModelPrice {
        version: String::new(),
        agent: "codex".into(),
        source_id: None,
        model: model.into(),
        currency: "CNY".into(),
        input: Some("9".into()),
        output: None,
        cache_read: None,
        cache_write: None,
        enabled: true,
        peak: None,
        peak_windows: None,
        long_context: None,
        source: None,
    }
}
#[test]
fn parses_cny_metadata_unknown_cache_and_explicit_free() {
    let c = price_import::parse(HTML).unwrap();
    assert_eq!(c.models.len(), 2);
    assert_eq!(c.fx_rate, "6.25");
    assert_eq!(c.fx_date, "2026-10-09");
    let p = price_import::match_price(&c, "synthetic-model-1").unwrap();
    assert_eq!(p.input.as_deref(), Some("2.125"));
    assert_eq!(p.output.as_deref(), Some("12.5"));
    assert!(p.cached.is_none());
    assert_eq!(
        price_import::match_price(&c, "synthetic-zero")
            .unwrap()
            .input
            .as_deref(),
        Some("0")
    );
}
#[test]
fn matching_is_exact_after_normalization_never_guesses_suffixes() {
    let c = price_import::parse(HTML).unwrap();
    assert!(price_import::match_price(&c, "openai/Synthetic-Model-1").is_some());
    for model in [
        "",
        "未知模型",
        "synthetic-model",
        "synthetic-model-1-256k",
        "unknown/synthetic-model-1",
    ] {
        assert!(price_import::match_price(&c, model).is_none());
    }
    let mut ambiguous = price_import::parse(HTML).unwrap();
    let mut duplicate = ambiguous.models[0].clone();
    duplicate.slug = "other-slug".into();
    ambiguous.models.push(duplicate);
    assert!(price_import::match_price(&ambiguous, "Synthetic Model 1").is_none());
}
#[test]
fn rejects_changed_page_and_oversized_input_without_executing_scripts() {
    assert!(price_import::parse("<script>throw new Error('do not execute')</script>").is_err());
    assert!(price_import::parse(&"x".repeat(2 * 1024 * 1024 + 1)).is_err());
    assert!(price_import::parse(
        &HTML
            .replace("inputCny", "inputChanged")
            .replace("outputCny", "outputChanged")
    )
    .is_err());
}
#[test]
fn import_is_idempotent_and_provenance_survives_reload() {
    let db = Database::open_in_memory().unwrap();
    let c = price_import::parse(HTML).unwrap();
    assert_eq!(
        price_import::apply(&db, &c, target("codex", "synthetic-model-1"))
            .unwrap()
            .imported,
        1
    );
    let rules = model_prices::list(&db).unwrap();
    assert_eq!(rules[0].currency, "CNY");
    assert!(rules[0].cache_write.is_none());
    assert!(rules[0].cache_read.is_none());
    assert_eq!(rules[0].source.as_ref().unwrap().fx_date, "2026-10-09");
    let version = rules[0].version.clone();
    let second = price_import::apply(&db, &c, target("codex", "synthetic-model-1")).unwrap();
    assert_eq!((second.imported, second.preserved), (0, 1));
    assert_eq!(model_prices::list(&db).unwrap()[0].version, version);
}
#[test]
fn manual_and_disabled_rules_are_never_overwritten() {
    let db = Database::open_in_memory().unwrap();
    let c = price_import::parse(HTML).unwrap();
    let saved = model_prices::save(&db, manual("synthetic-model-1")).unwrap();
    let result = price_import::apply(&db, &c, target("codex", "synthetic-model-1")).unwrap();
    assert_eq!((result.imported, result.preserved), (0, 1));
    assert_eq!(model_prices::list(&db).unwrap()[0].version, saved.version);
    model_prices::save(
        &db,
        ModelPrice {
            enabled: false,
            ..saved
        },
    )
    .unwrap();
    assert_eq!(
        price_import::apply(&db, &c, target("codex", "synthetic-model-1"))
            .unwrap()
            .preserved,
        1
    );
    assert!(model_prices::list(&db).unwrap().is_empty());
}
#[test]
fn manual_edit_clears_import_origin() {
    let db = Database::open_in_memory().unwrap();
    let c = price_import::parse(HTML).unwrap();
    price_import::apply(&db, &c, target("codex", "synthetic-model-1")).unwrap();
    let mut rule = model_prices::list(&db).unwrap().remove(0);
    rule.input = Some("42".into());
    let saved = model_prices::save(&db, rule).unwrap();
    assert!(saved.source.is_none());
    assert!(model_prices::list(&db).unwrap()[0].source.is_none());
}
#[test]
fn failed_batch_rolls_back() {
    let db = Database::open_in_memory().unwrap();
    let c = price_import::parse(HTML).unwrap();
    // BTreeSet visits codex before invalid_agent: fail after a real insert, then roll it back.
    db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES ('s','codex','local_log','/synthetic','test')",[]).unwrap();
    db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s','test','committed')",[]).unwrap();
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,observed_at,granularity,model_raw,tokens_quality) VALUES ('e',1,1,'s','codex','local_log','2026-10-12T10:00:00Z','request','synthetic-model-1','unknown')",[]).unwrap();
    assert!(price_import::apply(&db, &c, target("invalid_agent", "synthetic-model-1")).is_err());
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM price_versions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn refreshing_imported_prices_appends_history_without_changing_manual_rules() {
    let db = Database::open_in_memory().unwrap();
    let c = price_import::parse(HTML).unwrap();
    price_import::apply(&db, &c, target("codex", "synthetic-model-1")).unwrap();
    let first = model_prices::list(&db).unwrap().remove(0);
    let mut changed = c;
    changed
        .models
        .iter_mut()
        .find(|p| p.slug == "synthetic-model-1")
        .unwrap()
        .input = Some("3.5".into());
    assert_eq!(
        price_import::apply(&db, &changed, target("codex", "synthetic-model-1"))
            .unwrap()
            .imported,
        1
    );
    let second = model_prices::list(&db).unwrap().remove(0);
    assert_eq!(second.input.as_deref(), Some("3.5"));
    assert_ne!(first.version, second.version);
    let history = providock_lib::services::pricing::price_versions_list(&db).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history
            .iter()
            .find(|p| p.price_version == first.version)
            .unwrap()
            .rates
            .input
            .as_deref(),
        Some("2.125")
    );
}
#[test]
fn tiny_positive_rate_is_unknown_instead_of_free() {
    let c = price_import::parse(&HTML.replace("2.125", "0.00000000001")).unwrap();
    assert!(price_import::match_price(&c, "synthetic-model-1")
        .unwrap()
        .input
        .is_none());
}

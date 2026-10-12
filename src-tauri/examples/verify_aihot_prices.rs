//! Explicit read-only network check of the public price source; never uses an account or real usage.
fn main() {
    let catalogue =
        providock_lib::services::price_import::fetch().expect("download and parse AIHOT");
    println!(
        "models={} source_updated={} fx={} fx_date={}",
        catalogue.models.len(),
        catalogue.source_updated_at,
        catalogue.fx_rate,
        catalogue.fx_date
    );
    let db = providock_lib::storage::Database::open_in_memory().unwrap();
    let result = providock_lib::services::price_import::apply(
        &db,
        &catalogue,
        Some(providock_lib::services::price_import::ImportTarget {
            agent: "codex".into(),
            model: "gpt-6.1-sol".into(),
        }),
    )
    .unwrap();
    assert_eq!(result.imported, 1);
    let rules = providock_lib::services::model_prices::list(&db).unwrap();
    assert_eq!(rules[0].currency, "CNY");
    assert!(rules[0].cache_write.is_none());
    assert!(rules[0].source.is_some());
    println!("real-source -> isolated DB import verified");
}

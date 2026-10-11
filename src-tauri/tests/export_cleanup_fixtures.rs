//! T14 集成夹具测试：统计导出、数据清理、价格版本。
//!
//! 覆盖跨域、跨模块行为（用真实 ingestion 提交路径配合）：
//! - CSV 公式注入前缀、导出不含 project_ref 明文（脱敏为匿名标签）；
//! - 统计清理与账户清理两域互不影响；
//! - `clear` 后墓碑拦截重扫、`purge` 后同文件可重新导入；
//! - 账户彻底删除的历史快照保留/删除选项；
//! - 未知价格 None、价格版本区间选择、多币种不合并。
//!
//! 语义依据：SECURITY_PRIVACY 第 5/6 节、DATA_CONTRACTS 第 4/6/9 节、ROADMAP C15。

use providock_lib::domain::{
    AgentKind, Completeness, EvidenceRef, Granularity, Quality, StreamDomain, TokenSet, UsageEvent,
    USAGE_EVENT_SCHEMA_VERSION,
};
use providock_lib::ingestion::{
    commit_batch, start_parse_run, CursorUpdate, EventInput, ImportBatch,
};
use providock_lib::services::accounts::{
    account_create, credential_save, snapshot_write, BalanceEntry, BalanceKind, ConfigKind,
    CredentialType, NewAccount, NewSnapshot, QuotaUnit, QuotaWindow, UsageMetric,
};
use providock_lib::services::cleanup::{
    clear_account_snapshots, clear_statistics_detail, delete_account_fully, purge_statistics,
    PURGE_REIMPORT_HINT_KEY,
};
use providock_lib::services::export::{
    export_statistics, write_export, ExportFormat, ExportPathPolicy, CONTRACT_VERSION,
};
use providock_lib::services::pricing::{
    estimate_cost_at, insert_price_version, price_version_get, PriceVersion, Rates,
};
use providock_lib::storage::Database;
use providock_lib::vault::{MemoryVault, SecretString};
use std::path::PathBuf;

const FIXED_AT: &str = "2026-10-11T00:00:00Z";

fn new_db() -> Database {
    let db = Database::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO sources (source_id, agent, stream_domain, root_path, parser_version)
             VALUES ('src1', 'kimi_code', 'local_log', '/anon', 'kimi-wire-1.5')",
            [],
        )
        .unwrap();
    db
}

fn sample_event(id: &str, project_ref: Option<&str>, model_raw: Option<&str>) -> UsageEvent {
    UsageEvent {
        schema_version: USAGE_EVENT_SCHEMA_VERSION,
        event_id: id.into(),
        agent: AgentKind::KimiCode,
        source_id: "src1".into(),
        stream_domain: StreamDomain::LocalLog,
        session_id: Some("s1".into()),
        parent_session_id: None,
        actor_id: Some("main".into()),
        turn_id: None,
        project_ref: project_ref.map(str::to_string),
        occurred_at: Some("2026-10-11T06:00:00Z".into()),
        observed_at: "2026-10-11T06:00:01Z".into(),
        model_raw: model_raw.map(str::to_string),
        model_normalized: None,
        granularity: Granularity::Request,
        request_count: Some(1),
        tokens: TokenSet {
            input_fresh: Some(100),
            input_cache_read: Some(40),
            input_cache_write: Some(0),
            input_total: Some(140),
            output_total: Some(30),
            output_reasoning: None,
            total: Some(170),
            quality: Quality::Reported,
        },
        cost_reported: None,
        cost_estimated: None,
        completeness: Completeness::Complete,
        account_link: None,
        evidence_refs: vec![EvidenceRef {
            source_id: "src1".into(),
            file_identity: "fileA".into(),
            generation: 0,
            locator: "1".into(),
            parser_version: "kimi-wire-1.5".into(),
        }],
    }
}

fn import(db: &mut Database, event: UsageEvent) -> providock_lib::ingestion::CommitStats {
    let run = start_parse_run(db, "src1", "kimi-wire-1.5").unwrap();
    let batch = ImportBatch {
        source_id: "src1".into(),
        events: vec![EventInput {
            event,
            replace_existing: false,
        }],
        sessions: vec![],
        cursor: Some(CursorUpdate {
            file_identity: "fileA".into(),
            generation: 0,
            parser_version: "kimi-wire-1.5".into(),
            cursor_kind: "byte_offset".into(),
            cursor_value: "1".into(),
            tail_checksum: None,
        }),
    };
    commit_batch(db, run, &batch).unwrap()
}

fn count(db: &Database, table: &str) -> i64 {
    db.conn()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn temp_path(name: &str, ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "providock-t14-{}-{}.{ext}",
        std::process::id(),
        name
    ))
}

// ---------------------------------------------------------------------------
// A. 导出：脱敏、防公式注入、字段集合
// ---------------------------------------------------------------------------

#[test]
fn csv_text_is_prefixed_against_formula_injection_and_numeric_is_raw() {
    let mut db = new_db();
    // model_raw 以 = 开头（文本列 → 必须前缀）；金额为合法负数（数值列 → 原样）
    let mut ev = sample_event(
        "e-inject",
        Some("/Users/realuser/Secret Project"),
        Some("=cmd()"),
    );
    ev.cost_reported = Some(providock_lib::domain::CostInfo {
        money: providock_lib::domain::Money::new("-0.05", "USD").unwrap(),
        quality: Quality::Reported,
        completeness: Completeness::Complete,
        price_version: None,
        scope: None,
    });
    // 通过 ingestion 真实提交（cost 字段由 commit_batch 落库）
    let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
    commit_batch(
        &mut db,
        run,
        &ImportBatch {
            source_id: "src1".into(),
            events: vec![EventInput {
                event: ev,
                replace_existing: false,
            }],
            sessions: vec![],
            cursor: None,
        },
    )
    .unwrap();

    let out = temp_path("inject", "csv");
    let policy = ExportPathPolicy::new(&out);
    write_export(
        &db,
        &providock_lib::services::aggregate::UsageFilter::default(),
        ExportFormat::Csv,
        &policy,
    )
    .unwrap();
    let csv = std::fs::read_to_string(&out).unwrap();

    assert!(csv.contains("'=cmd()"), "文本公式需前缀单引号: {csv}");
    assert!(csv.contains("-0.05"), "负金额应原样: {csv}");
    assert!(!csv.contains("'-0.05"), "数值不得加注入前缀: {csv}");
    assert!(!csv.contains("realuser"), "不得含项目明文: {csv}");
    assert!(csv.contains("<file:"), "应为匿名标签: {csv}");
    std::fs::remove_file(&out).ok();
}

#[test]
fn preview_reports_fields_range_and_redaction_without_secret_columns() {
    let mut db = new_db();
    import(
        &mut db,
        sample_event("e1", Some("/Users/realuser/proj"), Some("kimi-k2")),
    );
    let policy = ExportPathPolicy::new(temp_path("preview", "json"));
    let preview = export_statistics(
        &db,
        &providock_lib::services::aggregate::UsageFilter::default(),
        ExportFormat::Json,
        &policy,
    )
    .unwrap();
    assert_eq!(preview.contract_version, CONTRACT_VERSION);
    assert_eq!(preview.record_count, 1);
    assert!(preview.fields.iter().any(|f| f.name == "project_ref"));
    assert!(preview
        .fields
        .iter()
        .any(|f| f.name == "total" && f.unit == "tokens"));
    // 导出字段集合不含凭据/正文列
    for f in &preview.fields {
        assert!(!f.name.contains("credential"));
        assert!(!f.name.contains("vault"));
        assert!(!f.name.contains("body"));
    }
    assert!(preview
        .note_keys
        .contains(&"export.redaction.project_paths_anonymized"));
    assert!(preview
        .note_keys
        .contains(&"export.redaction.no_credentials"));
}

#[test]
fn json_export_anonymizes_project_path_and_keeps_currencies_separate() {
    let mut db = new_db();
    let mut ev_usd = sample_event("e-usd", Some("/Users/realuser/usd"), Some("m"));
    ev_usd.cost_reported = Some(providock_lib::domain::CostInfo {
        money: providock_lib::domain::Money::new("1.25", "USD").unwrap(),
        quality: Quality::Reported,
        completeness: Completeness::Complete,
        price_version: None,
        scope: None,
    });
    let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
    commit_batch(
        &mut db,
        run,
        &ImportBatch {
            source_id: "src1".into(),
            events: vec![EventInput {
                event: ev_usd,
                replace_existing: false,
            }],
            sessions: vec![],
            cursor: None,
        },
    )
    .unwrap();
    // 第二个事件：不同币种
    let mut ev2 = sample_event("e-cny", Some("/Users/realuser/cny"), Some("m"));
    ev2.cost_reported = Some(providock_lib::domain::CostInfo {
        money: providock_lib::domain::Money::new("7.5", "CNY").unwrap(),
        quality: Quality::Reported,
        completeness: Completeness::Complete,
        price_version: None,
        scope: None,
    });
    let run = start_parse_run(&db, "src1", "kimi-wire-1.5").unwrap();
    commit_batch(
        &mut db,
        run,
        &ImportBatch {
            source_id: "src1".into(),
            events: vec![EventInput {
                event: ev2,
                replace_existing: false,
            }],
            sessions: vec![],
            cursor: None,
        },
    )
    .unwrap();

    let out = temp_path("json", "json");
    write_export(
        &db,
        &providock_lib::services::aggregate::UsageFilter::default(),
        ExportFormat::Json,
        &ExportPathPolicy::new(&out),
    )
    .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(doc["contract_version"], CONTRACT_VERSION);
    assert!(!text.contains("realuser"), "不得含项目明文");
    // 所有记录的项目引用都是匿名标签
    for rec in doc["records"].as_array().unwrap() {
        assert!(rec["project_ref"].as_str().unwrap().starts_with("<file:"));
    }
    // 币种分别列出，不合并
    let currencies: Vec<String> = doc["currencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(currencies, vec!["CNY".to_string(), "USD".to_string()]);
    assert_eq!(doc["coverage"]["events_total"], 2);
    std::fs::remove_file(&out).ok();
}

// ---------------------------------------------------------------------------
// B. 清理：墓碑/彻底清空、两域隔离、账户删除
// ---------------------------------------------------------------------------

#[test]
fn clear_keeps_tombstone_then_purge_allows_reimport() {
    let mut db = new_db();
    let stats = import(&mut db, sample_event("e1", None, Some("kimi-k2")));
    assert_eq!(stats.inserted, 1);
    assert_eq!(count(&db, "usage_events"), 1);

    // 清理明细：保留身份墓碑，游标保留
    let cleared = clear_statistics_detail(&mut db, "src1").unwrap();
    assert_eq!(cleared.events_removed, 1);
    assert_eq!(cleared.identities_retired, 1);
    assert_eq!(count(&db, "usage_events"), 0);
    assert_eq!(count(&db, "event_identity_tombstones"), 1);
    assert_eq!(count(&db, "ingestion_cursors"), 1);

    // 同文件重扫：墓碑拦截，不重新计入
    let rescan = import(&mut db, sample_event("e1", None, Some("kimi-k2")));
    assert_eq!(rescan.tombstoned, 1);
    assert_eq!(count(&db, "usage_events"), 0);

    // 彻底清空：墓碑、游标、parse_runs、会话一并删；提示可重新导入
    let purged = purge_statistics(&mut db, "src1").unwrap();
    assert_eq!(purged.tombstones_removed, 1);
    assert_eq!(purged.cursors_removed, 1);
    assert_eq!(purged.reimport_hint_key, PURGE_REIMPORT_HINT_KEY);
    assert_eq!(count(&db, "event_identity_tombstones"), 0);

    // purge 后同文件重扫可重新导入
    let reimport = import(&mut db, sample_event("e1", None, Some("kimi-k2")));
    assert_eq!(reimport.inserted, 1);
    assert_eq!(count(&db, "usage_events"), 1);
}

fn make_account_with_snapshot(db: &mut Database, name: &str) -> String {
    let account = account_create(db, &NewAccount::new("kimi", name, ConfigKind::ApiPayg)).unwrap();
    let balance = BalanceEntry::new(BalanceKind::Paid, "12.50", "USD").unwrap();
    let window = QuotaWindow::new("acct:tokens", QuotaUnit::Tokens);
    let metric = UsageMetric::new("lifetime_tokens", "1000", QuotaUnit::Tokens)
        .unwrap()
        .with_scope("account");
    snapshot_write(
        db,
        &NewSnapshot {
            account_id: account.account_id.clone(),
            credential_id: None,
            queried_at: "2026-10-11T06:00:00Z".into(),
            provider_as_of: None,
            evidence: None,
            expires_at: None,
            balances: vec![balance],
            quota_windows: vec![window],
            usage_metrics: vec![metric],
        },
    )
    .unwrap();
    account.account_id
}

#[test]
fn statistics_and_account_cleanup_do_not_touch_each_other() {
    let mut db = new_db();
    import(&mut db, sample_event("e1", None, Some("kimi-k2")));
    let account_id = make_account_with_snapshot(&mut db, "Main");
    assert_eq!(count(&db, "account_snapshots"), 1);
    assert_eq!(count(&db, "balances"), 1);
    assert_eq!(count(&db, "quota_windows"), 1);
    assert_eq!(count(&db, "usage_metrics"), 1);

    // 统计清理：账户侧表一行不动
    clear_statistics_detail(&mut db, "src1").unwrap();
    assert_eq!(count(&db, "usage_events"), 0);
    assert_eq!(count(&db, "event_identity_tombstones"), 1);
    assert_eq!(count(&db, "account_snapshots"), 1, "统计清理不得动账户快照");
    assert_eq!(count(&db, "balances"), 1);
    assert_eq!(count(&db, "quota_windows"), 1);
    assert_eq!(count(&db, "usage_metrics"), 1, "统计清理不得动账户指标");
    assert_eq!(count(&db, "accounts"), 1);

    // 账户清理：用量账本一行不动
    let cleared = clear_account_snapshots(&mut db, &account_id).unwrap();
    assert_eq!(cleared.snapshots_removed, 1);
    assert_eq!(cleared.balances_removed, 1);
    assert_eq!(cleared.quota_windows_removed, 1);
    assert_eq!(cleared.usage_metrics_removed, 1);
    assert_eq!(count(&db, "account_snapshots"), 0);
    assert_eq!(count(&db, "usage_metrics"), 0);
    assert_eq!(count(&db, "usage_events"), 0, "账户清理不得动用量账本");
    assert_eq!(
        count(&db, "event_identity_tombstones"),
        1,
        "账户清理不得动墓碑"
    );
    assert_eq!(count(&db, "accounts"), 1, "账户行保留");
}

#[test]
fn delete_account_fully_honors_history_option() {
    let vault = MemoryVault::new();
    let mut db = new_db();

    // keep_history = true：快照保留，凭据按引用清理
    let keep_id = make_account_with_snapshot(&mut db, "Keep");
    credential_save(
        &db,
        &vault,
        &keep_id,
        CredentialType::ApiKey,
        None,
        &SecretString::new("sk-synthetic-keep-0000"),
    )
    .unwrap();
    let result = delete_account_fully(&mut db, &vault, &keep_id, true).unwrap();
    assert!(result.history_kept);
    assert_eq!(result.snapshots_removed, 0);
    assert_eq!(result.credentials.refs_removed, 1);
    assert_eq!(result.credentials.vault_entries_removed, 1);
    assert_eq!(count(&db, "account_snapshots"), 1, "历史快照应保留");
    assert_eq!(count(&db, "credential_refs"), 0);

    // keep_history = false：快照一并删除
    let wipe_id = make_account_with_snapshot(&mut db, "Wipe");
    let result = delete_account_fully(&mut db, &vault, &wipe_id, false).unwrap();
    assert!(!result.history_kept);
    assert_eq!(result.snapshots_removed, 1);
    assert_eq!(count(&db, "account_snapshots"), 1, "只应删掉本账户的快照");
    assert_eq!(count(&db, "balances"), 1);
    assert_eq!(count(&db, "quota_windows"), 1);
}

// ---------------------------------------------------------------------------
// C. 价格版本
// ---------------------------------------------------------------------------

fn price(id: &str, from: Option<&str>, currency: &str, input: &str, output: &str) -> PriceVersion {
    PriceVersion {
        price_version: id.into(),
        provider: "kimi".into(),
        model_version: "kimi-k2".into(),
        currency: currency.into(),
        effective_from: from.map(str::to_string),
        effective_to: None,
        region: None,
        tier: None,
        rates: Rates {
            input: Some(input.into()),
            output: Some(output.into()),
            cache_read: Some("0".into()),
            cache_write: Some("0".into()),
            ..Default::default()
        },
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
fn unknown_price_is_none_and_version_selection_picks_latest() {
    let db = new_db();
    // 未知价格
    assert!(
        estimate_cost_at(&db, "kimi", "no-such-model", &tokens(), FIXED_AT)
            .unwrap()
            .is_none()
    );

    insert_price_version(
        &db,
        &price("v1", Some("2026-01-01T00:00:00Z"), "USD", "1", "2"),
    )
    .unwrap();
    insert_price_version(
        &db,
        &price("v2", Some("2026-09-01T00:00:00Z"), "USD", "10", "20"),
    )
    .unwrap();
    let cost = estimate_cost_at(&db, "kimi", "kimi-k2", &tokens(), FIXED_AT)
        .unwrap()
        .unwrap();
    assert_eq!(cost.price_version.as_deref(), Some("v2"));
    assert_eq!(cost.money.currency, "USD");
    assert_eq!(cost.money.amount, "14000"); // 1000*10 + 200*20

    // 价格版本不可变
    assert!(insert_price_version(
        &db,
        &price("v1", Some("2026-01-01T00:00:00Z"), "USD", "9", "9")
    )
    .is_err());
    assert_eq!(
        price_version_get(&db, "v1", "kimi", "kimi-k2")
            .unwrap()
            .unwrap()
            .rates
            .input
            .as_deref(),
        Some("1")
    );
}

#[test]
fn pricing_multi_currency_is_not_summed() {
    let db = new_db();
    insert_price_version(
        &db,
        &price("cny-v1", Some("2026-01-01T00:00:00Z"), "CNY", "7", "14"),
    )
    .unwrap();
    insert_price_version(
        &db,
        &price("usd-v1", Some("2026-01-01T00:00:00Z"), "USD", "1", "2"),
    )
    .unwrap();
    let cost = estimate_cost_at(&db, "kimi", "kimi-k2", &tokens(), FIXED_AT)
        .unwrap()
        .unwrap();
    // 只选其一（同 effective_from 时按 price_version 决定次序），绝不相加
    assert_eq!(cost.money.currency, "USD");
    assert_eq!(cost.money.amount, "1400");
}

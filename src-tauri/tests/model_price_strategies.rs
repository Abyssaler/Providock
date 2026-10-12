//! 手动价格策略（分时段 peak / 长上下文升档 longContext）的契约测试。
//!
//! 依据 DATA_CONTRACTS.md §6 与 2026-10-12 定价增量：
//! - 高峰判定用 Asia/Shanghai（固定 UTC+8）工作日 + 半开时段 [start,end)；法定节假日未实现。
//! - 长上下文按事件 input_total 整请求升档；判定所需 occurred_at / input_total 未知 → 估算未知。
//! - 策略随 rates_json.strategy 追加存储，list() 往返一致；无策略旧行行为不变。
use providock_lib::{
    domain::{Completeness, CostInfo, Quality, TokenSet},
    services::{
        aggregate::UsageFilter,
        model_prices::{self, ModelPrice},
        pricing::{self, LongContext, PeakWindows, PriceVersion, RateSet, Rates},
        usage_details,
    },
    storage::Database,
};

// 2026-10-12 是周一，2026-10-17 是周六；北京 = 固定 UTC+8。
const MON_0900: &str = "2026-10-12T01:00:00Z"; // 北京 09:00，时段起点（含）→ 高峰
const MON_1000: &str = "2026-10-12T02:00:00Z"; // 北京 10:00 → 高峰
const MON_1300: &str = "2026-10-12T05:00:00Z"; // 北京 13:00，时段间隙 → 低谷
const MON_1400: &str = "2026-10-12T06:00:00Z"; // 北京 14:00，时段起点（含）→ 高峰
const MON_1800: &str = "2026-10-12T10:00:00Z"; // 北京 18:00，时段终点（不含）→ 低谷
const SAT_1000: &str = "2026-10-17T02:00:00Z"; // 周六 10:00 → 低谷
const FRI_1630Z: &str = "2026-10-16T16:30:00Z"; // +8h 后落在周六 00:30 → 低谷

fn db() -> Database {
    let db = Database::open_in_memory().unwrap();
    db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES ('s1','codex','local_log','/synthetic','test')",[]).unwrap();
    db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s1','test','committed')",[]).unwrap();
    db
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

fn quoted(rule: &ModelPrice, at: Option<&str>, input_total: Option<u64>) -> Option<CostInfo> {
    model_prices::quote(rule, &tokens(), at, input_total).unwrap()
}

/// DeepSeek 风格：基础费率为低谷价，peak 为高峰价。
fn deepseek() -> ModelPrice {
    ModelPrice {
        source: None,
        version: String::new(),
        agent: "codex".into(),
        source_id: None,
        model: "deepseek-chat".into(),
        currency: "CNY".into(),
        input: Some("2".into()),
        output: Some("8".into()),
        cache_read: Some("1".into()),
        cache_write: None,
        enabled: true,
        peak: Some(RateSet {
            input: Some("4".into()),
            output: Some("16".into()),
            cache_read: Some("2".into()),
            cache_write: None,
        }),
        peak_windows: Some(PeakWindows {
            timezone: "Asia/Shanghai".into(),
            weekdays_only: true,
            ranges: vec!["09:00-12:00".into(), "14:00-18:00".into()],
        }),
        long_context: None,
    }
}

/// OpenAI 风格：基础费率为短上下文价，longContext 为整请求升档。
fn openai_style() -> ModelPrice {
    ModelPrice {
        source: None,
        version: String::new(),
        agent: "codex".into(),
        source_id: None,
        model: "gpt-5.2".into(),
        currency: "USD".into(),
        input: Some("1.75".into()),
        output: Some("14".into()),
        cache_read: Some("0.175".into()),
        cache_write: None,
        enabled: true,
        peak: None,
        peak_windows: None,
        long_context: Some(LongContext {
            input_over_tokens: 1500,
            input: Some("3.5".into()),
            output: Some("28".into()),
            cache_read: Some("0.35".into()),
            cache_write: None,
        }),
    }
}

#[test]
fn peak_windows_select_rates_by_beijing_weekday_and_half_open_ranges() {
    let rule = deepseek();
    // 高峰：1000*4 + 500*2 + 200*16 = 8200 /1M
    // 低谷：1000*2 + 500*1 + 200*8 = 4100 /1M
    for (at, expect) in [
        (MON_0900, "0.0082"),
        (MON_1000, "0.0082"),
        (MON_1400, "0.0082"),
        (MON_1300, "0.0041"),
        (MON_1800, "0.0041"),
        (SAT_1000, "0.0041"),
        (FRI_1630Z, "0.0041"), // 换算后才落到周六
    ] {
        let cost = quoted(&rule, Some(at), Some(1500)).expect(at);
        assert_eq!(cost.money.amount, expect, "{at}");
        assert_eq!(cost.completeness, Completeness::Complete, "{at}");
    }
    // 带偏移的 occurred_at 先归一到 UTC+8 再判定
    let cost = quoted(&rule, Some("2026-10-12T10:00:00+08:00"), Some(1500)).unwrap();
    assert_eq!(cost.money.amount, "0.0082");
    let cost = quoted(&rule, Some("2026-10-12T18:00:00+08:00"), Some(1500)).unwrap();
    assert_eq!(cost.money.amount, "0.0041");
    // 峰值估算仍标注同一规则版本（只是当前价比较，不写历史）
    assert_eq!(
        quoted(&rule, Some(MON_1000), Some(1500))
            .unwrap()
            .price_version,
        quoted(&rule, Some(MON_1300), Some(1500))
            .unwrap()
            .price_version
    );
}

#[test]
fn peak_rule_with_unknown_or_unparseable_time_is_unknown_not_base() {
    let rule = deepseek();
    assert!(
        quoted(&rule, None, Some(1500)).is_none(),
        "缺发生时间 ≠ 低谷"
    );
    assert!(quoted(&rule, Some("not-a-time"), Some(1500)).is_none());
    assert!(quoted(&rule, Some("2026-10-12"), Some(1500)).is_none());
    // 无策略规则不受时间影响
    let mut flat = deepseek();
    flat.peak = None;
    flat.peak_windows = None;
    assert!(quoted(&flat, None, Some(1500)).is_some());
}

#[test]
fn peak_strategy_validation() {
    let db = db();
    // peak 与 peakWindows 必须成对
    let mut r = deepseek();
    r.peak_windows = None;
    assert!(model_prices::save(&db, r).is_err());
    let mut r = deepseek();
    r.peak = None;
    assert!(model_prices::save(&db, r).is_err());
    // 时区白名单
    let mut r = deepseek();
    r.peak_windows.as_mut().unwrap().timezone = "UTC".into();
    assert!(model_prices::save(&db, r).is_err());
    // 时段数量 1–4
    let mut r = deepseek();
    r.peak_windows.as_mut().unwrap().ranges = vec![];
    assert!(model_prices::save(&db, r).is_err());
    let mut r = deepseek();
    r.peak_windows.as_mut().unwrap().ranges = vec![
        "08:00-09:00".into(),
        "09:00-10:00".into(),
        "10:00-11:00".into(),
        "11:00-12:00".into(),
        "12:00-13:00".into(),
    ];
    assert!(model_prices::save(&db, r).is_err());
    // 形状：HH:MM-HH:MM、合法钟点、start < end、不跨午夜、不重叠
    for bad in [
        vec!["9:00-12:00"],
        vec!["09:00-25:00"],
        vec!["09:60-12:00"],
        vec!["12:00-09:00"],
        vec!["22:00-02:00"],
        vec!["09:00-12:00", "11:00-13:00"],
        vec!["09:00-1200"],
    ] {
        let mut r = deepseek();
        r.peak_windows.as_mut().unwrap().ranges = bad.iter().map(|s| s.to_string()).collect();
        assert!(model_prices::save(&db, r).is_err(), "{bad:?}");
    }
    // 首尾相接不重叠，合法
    let mut r = deepseek();
    r.peak_windows.as_mut().unwrap().ranges = vec!["09:00-12:00".into(), "12:00-14:00".into()];
    assert!(model_prices::save(&db, r).is_ok());
    // 高峰费率与基础费率同一十进制校验；一档至少一项
    let mut r = deepseek();
    r.peak.as_mut().unwrap().input = Some("0.1234567891".into());
    assert!(model_prices::save(&db, r).is_err());
    let mut r = deepseek();
    r.peak = Some(RateSet::default());
    assert!(model_prices::save(&db, r).is_err());
    // 停用规则同样受校验
    let mut r = deepseek();
    r.enabled = false;
    r.peak = Some(RateSet::default());
    assert!(model_prices::save(&db, r).is_err());
}

#[test]
fn long_context_upgrades_whole_request_above_threshold_only() {
    let rule = openai_style();
    // 基础：1000*1.75 + 500*0.175 + 200*14 = 4637.5 /1M
    // 升档：1000*3.5 + 500*0.35 + 200*28 = 9275 /1M（输出同样按档位价）
    for (input_total, expect) in [(1499, "0.0046375"), (1500, "0.0046375"), (1501, "0.009275")] {
        let cost = quoted(&rule, Some(MON_1000), Some(input_total)).unwrap();
        assert_eq!(cost.money.amount, expect, "input_total={input_total}");
        assert_eq!(cost.completeness, Completeness::Complete);
    }
    // input_total 未知 → 整体未知（不是按基础费率）
    assert!(quoted(&rule, Some(MON_1000), None).is_none());
    assert!(quoted(&rule, None, None).is_none());
    // 档位内某分量费率未知 → 该分量未知，完整性降级但其余分量照常计价
    let mut partial = openai_style();
    partial.long_context.as_mut().unwrap().output = None;
    let cost = quoted(&partial, Some(MON_1000), Some(1501)).unwrap();
    assert_eq!(cost.money.amount, "0.003675"); // 3500 + 175，输出未知不计
    assert_eq!(cost.completeness, Completeness::Partial);
}

#[test]
fn long_context_validation() {
    let db = db();
    let mut r = openai_style();
    r.long_context.as_mut().unwrap().input_over_tokens = 0;
    assert!(model_prices::save(&db, r).is_err());
    let mut r = openai_style();
    r.long_context.as_mut().unwrap().input_over_tokens = i64::MAX as u64 + 1;
    assert!(model_prices::save(&db, r).is_err());
    let mut r = openai_style();
    r.long_context = Some(LongContext {
        input_over_tokens: 1000,
        input: None,
        output: None,
        cache_read: None,
        cache_write: None,
    });
    assert!(model_prices::save(&db, r).is_err());
    let mut r = openai_style();
    r.long_context.as_mut().unwrap().input = Some("-0.5".into());
    assert!(model_prices::save(&db, r).is_err());
}

#[test]
fn long_context_wins_over_peak_and_peak_never_combines_with_tier() {
    // 两种策略同配：超过阈值 → 档位费率（无高峰变体，不叠加）；
    // 未超阈值 → 基础费率（本实现中 peak 只在未配置 longContext 时参与）。
    let mut rule = openai_style();
    rule.peak = Some(RateSet {
        input: Some("9".into()),
        output: Some("9".into()),
        cache_read: Some("9".into()),
        cache_write: None,
    });
    rule.peak_windows = Some(PeakWindows {
        timezone: "Asia/Shanghai".into(),
        weekdays_only: true,
        ranges: vec!["09:00-12:00".into()],
    });
    let db = db();
    assert!(model_prices::save(&db, rule.clone()).is_ok());
    let over = quoted(&rule, Some(MON_1000), Some(1501)).unwrap();
    assert_eq!(over.money.amount, "0.009275", "升档优先于高峰");
    let under = quoted(&rule, Some(MON_1000), Some(1500)).unwrap();
    assert_eq!(
        under.money.amount, "0.0046375",
        "升档规则下高峰不叠加到基础档"
    );
}

#[test]
fn strategies_roundtrip_through_rates_json_and_old_rows_stay_flat() {
    let db = db();
    let deepseek = model_prices::save(&db, deepseek()).unwrap();
    let openai = model_prices::save(&db, openai_style()).unwrap();
    // 存储形状：策略在 rates_json.strategy 内，tier 字符串不变
    let json: String = db
        .conn()
        .query_row(
            "SELECT rates_json FROM price_versions WHERE price_version=?1",
            [&deepseek.version],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        json,
        r#"{"input":"2","output":"8","cache_read":"1","unit":"per_million_tokens","strategy":{"peak":{"input":"4","output":"16","cacheRead":"2"},"peakWindows":{"timezone":"Asia/Shanghai","weekdaysOnly":true,"ranges":["09:00-12:00","14:00-18:00"]}}}"#
    );
    let tier: String = db
        .conn()
        .query_row(
            "SELECT tier FROM price_versions WHERE price_version=?1",
            [&openai.version],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tier, "providock_manual_per_million_v1");
    // list() 往返一致
    let rules = model_prices::list(&db).unwrap();
    assert_eq!(rules.len(), 2);
    for saved in [&deepseek, &openai] {
        let listed = rules.iter().find(|r| r.model == saved.model).unwrap();
        assert_eq!(listed, saved);
    }
    // 旧版扁平 v1 行（rates_json 无 strategy 键）照常 list/quote
    pricing::insert_price_version(
        &db,
        &PriceVersion {
            price_version: "manual-legacy-flat".into(),
            provider: "manual:codex".into(),
            model_version: "m1".into(),
            currency: "USD".into(),
            effective_from: Some("2026-01-01T00:00:00Z".into()),
            effective_to: None,
            region: None,
            tier: Some("providock_manual_per_million_v1".into()),
            rates: Rates {
                input: Some("3".into()),
                output: Some("12".into()),
                unit: Some("per_million_tokens".into()),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let raw: String = db
        .conn()
        .query_row(
            "SELECT rates_json FROM price_versions WHERE price_version='manual-legacy-flat'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!raw.contains("strategy"), "无策略行的 JSON 形状不变: {raw}");
    let rules = model_prices::list(&db).unwrap();
    let legacy = rules.iter().find(|r| r.model == "m1").unwrap();
    assert!(
        legacy.peak.is_none() && legacy.peak_windows.is_none() && legacy.long_context.is_none()
    );
    // 1000*3 + 200*12 = 5400 /1M；cache_read 费率未知 → 部分估算
    let cost = quoted(legacy, Some(MON_1000), Some(1500)).unwrap();
    assert_eq!(cost.money.amount, "0.0054");
    assert_eq!(cost.completeness, Completeness::Partial);
}

fn event(db: &Database, id: &str, model: &str, at: Option<&str>, input_total: Option<i64>) {
    let total = input_total.map(|i| i + 200);
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,model_raw,input_fresh,input_cache_read,input_cache_write,input_total,output_total,total,tokens_quality) VALUES (?1,1,1,'s1','codex','local_log',?2,'2026-10-12T00:00:00Z','request',?3,1000,500,0,?4,200,?5,'reported')",rusqlite::params![id,at,model,input_total,total]).unwrap();
}

#[test]
fn same_model_peak_and_offpeak_events_estimate_differently_in_dashboard() {
    let db = db();
    event(&db, "e-peak", "deepseek-chat", Some(MON_1000), Some(1500));
    event(&db, "e-off", "deepseek-chat", Some(MON_1300), Some(1500));
    event(&db, "e-no-time", "deepseek-chat", None, Some(1500));
    model_prices::save(&db, deepseek()).unwrap();

    let d = usage_details::dashboard(&db, &UsageFilter::default(), true).unwrap();
    assert_eq!(d.priced_events, 2, "缺发生时间的事件保持未知");
    assert_eq!(d.complete_cost_events, 2);
    let cny = d.current_cost.iter().find(|a| a.currency == "CNY").unwrap();
    assert_eq!(cny.amount, "0.0123"); // 0.0082 + 0.0041

    let page = usage_details::events(&db, &UsageFilter::default(), 1, 10).unwrap();
    let amount = |id: &str| {
        page.rows
            .iter()
            .find(|e| e.event_id == id)
            .unwrap()
            .cost_calculated
            .as_ref()
            .map(|c| c.amount.as_str())
    };
    assert_eq!(amount("e-peak"), Some("0.0082"));
    assert_eq!(amount("e-off"), Some("0.0041"));
    assert_eq!(amount("e-no-time"), None);
}

#[test]
fn long_context_tier_uses_event_input_total_in_dashboard() {
    let db = db();
    event(&db, "e-short", "gpt-5.2", Some(MON_1000), Some(1500)); // 阈值 1500，等于不升档
    event(&db, "e-long", "gpt-5.2", Some(MON_1000), Some(1501));
    event(&db, "e-unknown", "gpt-5.2", Some(MON_1000), None);
    model_prices::save(&db, openai_style()).unwrap();

    let d = usage_details::dashboard(&db, &UsageFilter::default(), true).unwrap();
    assert_eq!(d.priced_events, 2, "input_total 未知的事件保持未知");
    let usd = d.current_cost.iter().find(|a| a.currency == "USD").unwrap();
    assert_eq!(usd.amount, "0.0139125"); // 0.0046375 + 0.009275

    let page = usage_details::events(&db, &UsageFilter::default(), 1, 10).unwrap();
    let amount = |id: &str| {
        page.rows
            .iter()
            .find(|e| e.event_id == id)
            .unwrap()
            .cost_calculated
            .as_ref()
            .map(|c| c.amount.as_str())
    };
    assert_eq!(amount("e-short"), Some("0.0046375"));
    assert_eq!(amount("e-long"), Some("0.009275"));
    assert_eq!(amount("e-unknown"), None);
}

#[test]
fn per_million_to_per_token_shift_is_exact_at_one_million_tokens() {
    let db = db();
    // 100 万输入 tokens 的事件：高峰 ¥4/1M → 恰 ¥4
    db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,model_raw,input_fresh,input_cache_read,input_cache_write,input_total,output_total,total,tokens_quality) VALUES ('e-1m',1,1,'s1','codex','local_log',?1,'2026-10-12T00:00:00Z','request','deepseek-chat',1000000,0,0,1000000,0,1000000,'reported')",rusqlite::params![MON_1000]).unwrap();
    model_prices::save(&db, deepseek()).unwrap();
    let page = usage_details::events(&db, &UsageFilter::default(), 1, 10).unwrap();
    let cost = page.rows[0].cost_calculated.as_ref().unwrap();
    assert_eq!(cost.amount, "4");
    assert!(cost.complete, "cache_write 数量为 0 时费率未知不影响完整性");
    let d = usage_details::dashboard(&db, &UsageFilter::default(), true).unwrap();
    assert_eq!(d.current_cost[0].amount, "4");
}

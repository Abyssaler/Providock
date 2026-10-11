//! B01–B04 公开官方 HTTP 账户适配器夹具驱动测试。
//!
//! 覆盖 DeepSeek / OpenRouter（当前 Key + Credits）/ Novita 的请求构造、正常解析、
//! 单位换算（Novita 1/10000 USD）、多币种、负余额、total 不重复加、401/403/429/5xx、
//! HTTP 200 业务错误与 schema 变化不补零。
//!
//! 预期值由人工按证据语义独立计算（见 fixtures/accounts/<provider>/README.md），
//! 不由 parser 输出反推。

use providock_lib::adapters::accounts::{deepseek, novita, openrouter};
use providock_lib::services::accounts::{
    grouped_balances, BalanceEntry, CredentialType, FailureSignal, QuotaWindow,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 独立测试模块：加 `deepseek_openrouter_novita` 前缀，避免与共享账户测试文件的用例名混淆。
mod deepseek_openrouter_novita {
    use super::*;

// ---------------------------------------------------------------------------
// 夹具驱动
// ---------------------------------------------------------------------------

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/accounts")
}

fn case_dirs(root: &Path) -> Vec<String> {
    let mut cases: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.is_dir()
                .then(|| path.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    cases
}

fn load(case_dir: &Path) -> (String, Value) {
    let body = std::fs::read_to_string(case_dir.join("response.json")).unwrap();
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(case_dir.join("expected.json")).unwrap())
            .unwrap();
    (body, expected)
}

enum Outcome<'a> {
    Success {
        balances: &'a [BalanceEntry],
        windows: &'a [QuotaWindow],
        provider_as_of: &'a Option<String>,
    },
    Failure(&'a FailureSignal),
}

fn check(case: &str, outcome: Outcome, expected: &Value) {
    match expected["outcome"].as_str().unwrap() {
        "success" => match outcome {
            Outcome::Success {
                balances,
                windows,
                provider_as_of,
            } => {
                match &expected["provider_as_of"] {
                    Value::Null => assert!(provider_as_of.is_none(), "{case}: provider_as_of"),
                    Value::String(s) => assert_eq!(
                        provider_as_of.as_deref(),
                        Some(s.as_str()),
                        "{case}: provider_as_of"
                    ),
                    other => panic!("{case}: bad provider_as_of {other}"),
                }
                let exp_balances = expected["balances"].as_array().unwrap();
                assert_eq!(balances.len(), exp_balances.len(), "{case}: balance count");
                for (i, exp) in exp_balances.iter().enumerate() {
                    assert_balance(&balances[i], exp, &format!("{case}.balances[{i}]"));
                }
                let exp_windows = expected["quota_windows"].as_array().unwrap();
                assert_eq!(windows.len(), exp_windows.len(), "{case}: window count");
                for (i, exp) in exp_windows.iter().enumerate() {
                    assert_window(&windows[i], exp, &format!("{case}.quota_windows[{i}]"));
                }
            }
            Outcome::Failure(signal) => panic!("{case}: expected success, got {signal:?}"),
        },
        "failure" => match outcome {
            Outcome::Failure(signal) => {
                assert_eq!(
                    signal.status().as_str(),
                    expected["status"].as_str().unwrap(),
                    "{case}: status"
                );
                match &expected["business_code"] {
                    Value::Null => {
                        assert!(signal.business_code.is_none(), "{case}: business_code {:?}", signal.business_code)
                    }
                    Value::String(s) => assert_eq!(
                        signal.business_code.as_deref(),
                        Some(s.as_str()),
                        "{case}: business_code"
                    ),
                    other => panic!("{case}: bad business_code {other}"),
                }
                assert_eq!(
                    signal.schema_changed,
                    expected["schema_changed"].as_bool().unwrap(),
                    "{case}: schema_changed"
                );
                assert_eq!(
                    signal.transport_failed,
                    expected["transport_failed"].as_bool().unwrap_or(false),
                    "{case}: transport_failed"
                );
                assert_eq!(
                    signal.unsupported,
                    expected["unsupported"].as_bool().unwrap_or(false),
                    "{case}: unsupported"
                );
            }
            Outcome::Success { .. } => panic!("{case}: expected failure, got success"),
        },
        other => panic!("{case}: unknown outcome {other}"),
    }
}

fn assert_balance(actual: &BalanceEntry, exp: &Value, ctx: &str) {
    assert_eq!(actual.kind.as_str(), exp["kind"].as_str().unwrap(), "{ctx}: kind");
    assert_eq!(actual.amount, exp["amount"].as_str().unwrap(), "{ctx}: amount");
    assert_eq!(actual.currency, exp["currency"].as_str().unwrap(), "{ctx}: currency");
    match &exp["formula"] {
        Value::Null => assert!(
            actual.formula.is_none(),
            "{ctx}: formula expected null, got {:?}",
            actual.formula
        ),
        Value::String(s) => assert_eq!(actual.formula.as_deref(), Some(s.as_str()), "{ctx}: formula"),
        other => panic!("{ctx}: bad formula {other}"),
    }
    assert_eq!(actual.quality.as_str(), exp["quality"].as_str().unwrap(), "{ctx}: quality");
}

fn assert_window(actual: &QuotaWindow, exp: &Value, ctx: &str) {
    assert_eq!(actual.window_id, exp["window_id"].as_str().unwrap(), "{ctx}: window_id");
    assert_eq!(actual.unit.as_str(), exp["unit"].as_str().unwrap(), "{ctx}: unit");
    cmp_opt_str(&actual.scope, &exp["scope"], &format!("{ctx}: scope"));
    cmp_opt_str(&actual.label, &exp["label"], &format!("{ctx}: label"));
    match &exp["period_kind"] {
        Value::Null => assert!(actual.period_kind.is_none(), "{ctx}: period_kind expected null"),
        Value::String(s) => assert_eq!(
            actual.period_kind.map(|p| p.as_str()),
            Some(s.as_str()),
            "{ctx}: period_kind"
        ),
        other => panic!("{ctx}: bad period_kind {other}"),
    }
    match &exp["duration_seconds"] {
        Value::Null => assert!(
            actual.duration_seconds.is_none(),
            "{ctx}: duration_seconds expected null"
        ),
        Value::Number(n) => assert_eq!(
            actual.duration_seconds,
            Some(n.as_u64().unwrap()),
            "{ctx}: duration_seconds"
        ),
        other => panic!("{ctx}: bad duration_seconds {other}"),
    }
    cmp_opt_str(&actual.used, &exp["used"], &format!("{ctx}: used"));
    cmp_opt_str(&actual.limit, &exp["limit"], &format!("{ctx}: limit"));
    cmp_opt_str(&actual.remaining, &exp["remaining"], &format!("{ctx}: remaining"));
}

fn cmp_opt_str(actual: &Option<String>, exp: &Value, ctx: &str) {
    match exp {
        Value::Null => assert!(actual.is_none(), "{ctx}: expected null, got {actual:?}"),
        Value::String(s) => assert_eq!(actual.as_deref(), Some(s.as_str()), "{ctx}"),
        other => panic!("{ctx}: bad expected {other}"),
    }
}

#[test]
fn deepseek_fixtures() {
    let root = fixtures_root().join("deepseek");
    let cases = case_dirs(&root);
    assert!(cases.len() >= 4, "deepseek fixture cases missing: {cases:?}");
    for case in cases {
        let (body, expected) = load(&root.join(&case));
        let status = expected["http_status"].as_u64().unwrap() as u16;
        match deepseek::parse_response(status, &body) {
            deepseek::ParsedAccountResponse::Success {
                balances,
                quota_windows,
                provider_as_of,
            } => check(
                &case,
                Outcome::Success {
                    balances: &balances,
                    windows: &quota_windows,
                    provider_as_of: &provider_as_of,
                },
                &expected,
            ),
            deepseek::ParsedAccountResponse::Failure(signal) => {
                check(&case, Outcome::Failure(&signal), &expected)
            }
        }
    }
}

#[test]
fn openrouter_fixtures() {
    let root = fixtures_root().join("openrouter");
    let cases = case_dirs(&root);
    assert!(cases.len() >= 5, "openrouter fixture cases missing: {cases:?}");
    for case in cases {
        let (body, expected) = load(&root.join(&case));
        let status = expected["http_status"].as_u64().unwrap() as u16;
        let endpoint = match expected["endpoint"].as_str().unwrap() {
            "current_key" => openrouter::Endpoint::CurrentKey,
            "credits" => openrouter::Endpoint::Credits,
            other => panic!("{case}: bad endpoint {other}"),
        };
        match openrouter::parse_response(endpoint, status, &body) {
            openrouter::ParsedAccountResponse::Success {
                balances,
                quota_windows,
                provider_as_of,
            } => check(
                &case,
                Outcome::Success {
                    balances: &balances,
                    windows: &quota_windows,
                    provider_as_of: &provider_as_of,
                },
                &expected,
            ),
            openrouter::ParsedAccountResponse::Failure(signal) => {
                check(&case, Outcome::Failure(&signal), &expected)
            }
        }
    }
}

#[test]
fn novita_fixtures() {
    let root = fixtures_root().join("novita");
    let cases = case_dirs(&root);
    assert!(cases.len() >= 4, "novita fixture cases missing: {cases:?}");
    for case in cases {
        let (body, expected) = load(&root.join(&case));
        let status = expected["http_status"].as_u64().unwrap() as u16;
        match novita::parse_response(status, &body) {
            novita::ParsedAccountResponse::Success {
                balances,
                quota_windows,
                provider_as_of,
            } => check(
                &case,
                Outcome::Success {
                    balances: &balances,
                    windows: &quota_windows,
                    provider_as_of: &provider_as_of,
                },
                &expected,
            ),
            novita::ParsedAccountResponse::Failure(signal) => {
                check(&case, Outcome::Failure(&signal), &expected)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 请求构造：method / host / path / 认证头形状 / 不含秘密；host 精确匹配
// ---------------------------------------------------------------------------

#[test]
fn request_specs_are_exact_and_carry_no_secret() {
    let ds = deepseek::build_request(&deepseek::RequestConfig::canonical()).unwrap();
    assert_eq!(ds.method, "GET");
    assert_eq!(ds.host, "api.deepseek.com");
    assert_eq!(ds.path, "/user/balance");
    assert_eq!(ds.url(), "https://api.deepseek.com/user/balance");
    let auth = ds.auth.as_ref().unwrap();
    assert_eq!(auth.header, "Authorization");
    assert_eq!(auth.scheme, "Bearer ");
    assert!(ds.headers.is_empty(), "DeepSeek 无额外静态头");
    assert!(ds
        .auth_header_placeholder()
        .unwrap()
        .contains("<secret-injected-by-service>"));

    let key = openrouter::build_request(&openrouter::RequestConfig::canonical(
        openrouter::Endpoint::CurrentKey,
    ))
    .unwrap();
    assert_eq!(key.method, "GET");
    assert_eq!(key.host, "openrouter.ai");
    assert_eq!(key.path, "/api/v1/key");
    let credits = openrouter::build_request(&openrouter::RequestConfig::canonical(
        openrouter::Endpoint::Credits,
    ))
    .unwrap();
    assert_eq!(credits.path, "/api/v1/credits");

    let nv = novita::build_request(&novita::RequestConfig::canonical()).unwrap();
    assert_eq!(nv.method, "GET");
    assert_eq!(nv.host, "api.novita.ai");
    assert_eq!(nv.path, "/openapi/v1/billing/balance/detail");
    assert!(nv
        .headers
        .iter()
        .any(|(k, v)| k == "Content-Type" && v == "application/json"));

    // 静态头里绝不能出现任何 Bearer/密钥值
    for headers in [&ds.headers, &key.headers, &credits.headers, &nv.headers] {
        for (_, value) in headers {
            assert!(!value.to_lowercase().contains("bearer"));
        }
    }
}

#[test]
fn hosts_must_match_exactly_not_by_substring() {
    assert!(deepseek::build_request(&deepseek::RequestConfig::new(
        "api.deepseek.com.evil.com"
    ))
    .is_err());
    assert!(deepseek::build_request(&deepseek::RequestConfig::new("evil-api.deepseek.com")).is_err());
    assert!(openrouter::build_request(&openrouter::RequestConfig::new(
        "openrouter.ai.evil.com",
        openrouter::Endpoint::CurrentKey
    ))
    .is_err());
    assert!(novita::build_request(&novita::RequestConfig::new("api.novita.ai.evil.com")).is_err());

    // 规范化到精确 host 的 URL 应被接受
    assert!(deepseek::build_request(&deepseek::RequestConfig::new("https://api.deepseek.com")).is_ok());
    assert!(novita::build_request(&novita::RequestConfig::new("https://api.novita.ai/")).is_ok());
}

// ---------------------------------------------------------------------------
// 失败映射：401/403 → auth_required，429 → rate_limited，5xx → unavailable
// ---------------------------------------------------------------------------

trait FailureStatus {
    fn failure_status(&self) -> &'static str;
}

impl FailureStatus for deepseek::ParsedAccountResponse {
    fn failure_status(&self) -> &'static str {
        match self {
            Self::Failure(signal) => signal.status().as_str(),
            Self::Success { .. } => panic!("expected failure"),
        }
    }
}

impl FailureStatus for openrouter::ParsedAccountResponse {
    fn failure_status(&self) -> &'static str {
        match self {
            Self::Failure(signal) => signal.status().as_str(),
            Self::Success { .. } => panic!("expected failure"),
        }
    }
}

impl FailureStatus for novita::ParsedAccountResponse {
    fn failure_status(&self) -> &'static str {
        match self {
            Self::Failure(signal) => signal.status().as_str(),
            Self::Success { .. } => panic!("expected failure"),
        }
    }
}

#[test]
fn error_statuses_map_to_contract_states() {
    assert_eq!(deepseek::parse_response(401, "{}").failure_status(), "auth_required");
    assert_eq!(deepseek::parse_response(403, "{}").failure_status(), "auth_required");
    assert_eq!(
        openrouter::parse_response(openrouter::Endpoint::Credits, 403, "{}").failure_status(),
        "auth_required"
    );
    assert_eq!(novita::parse_response(429, "{}").failure_status(), "rate_limited");
    assert_eq!(deepseek::parse_response(500, "{}").failure_status(), "unavailable");
    assert_eq!(
        openrouter::parse_response(openrouter::Endpoint::CurrentKey, 502, "{}").failure_status(),
        "unavailable"
    );
}

#[test]
fn schema_change_is_incompatible_and_never_zero_filled() {
    // 缺必需字段 → incompatible，且不产生 0 余额
    match openrouter::parse_response(openrouter::Endpoint::Credits, 200, r#"{"data":{}}"#) {
        openrouter::ParsedAccountResponse::Failure(signal) => {
            assert!(signal.schema_changed);
            assert_eq!(signal.status().as_str(), "incompatible");
        }
        other => panic!("expected incompatible, got {other:?}"),
    }
    match novita::parse_response(200, r#"{"availableBalance":"1.5"}"#) {
        novita::ParsedAccountResponse::Failure(signal) => assert!(signal.schema_changed),
        other => panic!("expected incompatible, got {other:?}"),
    }
    match deepseek::parse_response(200, r#"{"balance_infos":[{"currency":"CNY"}]}"#) {
        deepseek::ParsedAccountResponse::Failure(signal) => assert!(signal.schema_changed),
        other => panic!("expected incompatible, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 单位换算 / 多币种 / 负余额 / total 不重复加
// ---------------------------------------------------------------------------

#[test]
fn novita_units_are_exact_integer_division() {
    let body = r#"{"availableBalance":"1","cashBalance":"99999","creditLimit":"0","pendingCharges":"0","outstandingInvoices":"0"}"#;
    match novita::parse_response(200, body) {
        novita::ParsedAccountResponse::Success { balances, .. } => {
            assert_eq!(balances[0].amount, "0.0001");
            assert_eq!(balances[1].amount, "9.9999");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn negative_available_is_preserved() {
    let body = r#"{"availableBalance":"-5000","cashBalance":"0","creditLimit":"0","pendingCharges":"0","outstandingInvoices":"0"}"#;
    match novita::parse_response(200, body) {
        novita::ParsedAccountResponse::Success { balances, .. } => {
            assert_eq!(balances[0].amount, "-0.5");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn deepseek_total_does_not_double_count_components() {
    let body = r#"{"is_available":true,"balance_infos":[
        {"currency":"USD","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}"#;
    match deepseek::parse_response(200, body) {
        deepseek::ParsedAccountResponse::Success { balances, .. } => {
            let totals = grouped_balances(&balances).unwrap();
            assert_eq!(totals.len(), 1);
            assert_eq!(totals[0].currency, "USD");
            assert_eq!(totals[0].amount, "110.00", "total 已含赠金+充值，不得再相加");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn deepseek_multi_currency_is_never_added_across_currencies() {
    let body = r#"{"is_available":true,"balance_infos":[
        {"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"},
        {"currency":"USD","total_balance":"25.50","granted_balance":"0.00","topped_up_balance":"25.50"}]}"#;
    match deepseek::parse_response(200, body) {
        deepseek::ParsedAccountResponse::Success { balances, .. } => {
            let totals = grouped_balances(&balances).unwrap();
            assert_eq!(totals.len(), 2, "CNY 与 USD 分开，不跨币种合并");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn openrouter_key_budget_is_not_cash_balance() {
    let body = r#"{"data":{"limit":100,"limit_remaining":74.25,"usage":25.75,"limit_reset":"monthly"}}"#;
    match openrouter::parse_response(openrouter::Endpoint::CurrentKey, 200, body) {
        openrouter::ParsedAccountResponse::Success { balances, quota_windows, .. } => {
            assert!(balances.is_empty(), "Key 预算不是现金余额");
            assert!(quota_windows
                .iter()
                .all(|w| w.scope.as_deref() == Some("key")));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 凭据类型：B03 Credits 需要 management_key
// ---------------------------------------------------------------------------

#[test]
fn credential_types_match_evidence() {
    assert_eq!(deepseek::CREDENTIAL_TYPE, CredentialType::ApiKey);
    assert_eq!(novita::CREDENTIAL_TYPE, CredentialType::ApiKey);
    assert_eq!(
        openrouter::Endpoint::CurrentKey.credential_type(),
        CredentialType::ApiKey
    );
    assert_eq!(
        openrouter::Endpoint::Credits.credential_type(),
        CredentialType::ManagementKey
    );
}

} // mod deepseek_openrouter_novita

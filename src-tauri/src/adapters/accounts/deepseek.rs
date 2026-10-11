//! B01 DeepSeek API 余额适配器（纯请求构造 + 纯响应解析；不触网、不读凭据）。
//!
//! 证据：docs/research/evidence/accounts-b01-b07-candidates.md「B01 DeepSeek API 余额」（E1）；
//! 规范：docs/specs/ACCOUNT_ADAPTERS.md 第 1/5/6/7 节、docs/specs/DATA_CONTRACTS.md 第 7 节。
//!
//! 接口：`GET https://api.deepseek.com/user/balance`，`Authorization: Bearer <API_KEY>`。
//! 响应 `balance_infos[]` 每项含 `currency` / `total_balance` / `granted_balance` /
//! `topped_up_balance`；`total_balance` **已含**赠金与充值（`total = granted + topped_up`），
//! 因此 available 行注明公式，UI 端不得再把 grant / paid 相加。金额按币种分开，绝不跨币种合计。
//! `is_available=false` 是“账户当前无可用余额”的正常应答，不是查询失败。
//!
//! 请求/响应类型与十进制工具统一取自 [`crate::adapters::accounts::common`]。

use crate::adapters::accounts::common::{self, RequestSpec};
// 公共类型在本模块的再导出：适配器不再各自定义，旧的引用路径保持不变。
pub use crate::adapters::accounts::common::{AuthPlacement, ParsedAccountResponse, RequestConfig};
use crate::services::accounts::{
    BalanceEntry, BalanceKind, CredentialType, FailureSignal, SnapshotStatus,
};
use serde_json::Value;

/// 官方文档规定的精确 host（比较必须精确，禁止“包含该串”的域名）。
pub const CANONICAL_HOST: &str = "api.deepseek.com";
/// 固定路径。
pub const REQUEST_PATH: &str = "/user/balance";
/// 该接口接受的凭据类型（Bearer API Key）。
pub const CREDENTIAL_TYPE: CredentialType = CredentialType::ApiKey;
/// 余额接口超时提示（秒）；服务层可覆盖。
pub const TIMEOUT_HINT_SECONDS: u64 = common::DEFAULT_TIMEOUT_SECONDS;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestBuildError {
    #[error("deepseek host not allowed: expected {expected}, got {got}")]
    HostNotAllowed { expected: &'static str, got: String },
}

/// 构造请求。配置 host 解析出的域名必须**精确等于** [`CANONICAL_HOST`]。
pub fn build_request(cfg: &RequestConfig) -> Result<RequestSpec, RequestBuildError> {
    if common::extract_host(&cfg.host) != Some(CANONICAL_HOST) {
        return Err(RequestBuildError::HostNotAllowed {
            expected: CANONICAL_HOST,
            got: cfg.host.clone(),
        });
    }
    Ok(RequestSpec::https(CANONICAL_HOST, REQUEST_PATH).with_auth(AuthPlacement::bearer()))
}

/// 解析响应。非 200 交给 [`FailureSignal`] 默认映射（401/403→auth_required、
/// 429→rate_limited、5xx→unavailable）；HTTP 200 的业务错误与 schema 变化按失败处理，不补零。
pub fn parse_response(status: u16, body: &str) -> ParsedAccountResponse {
    if status != 200 {
        return ParsedAccountResponse::Failure(failure_from_status(status));
    }
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    };
    // HTTP 200 + 业务错误体（DeepSeek 错误形如 {"error":{"code":"...","message":"..."}}）。
    if let Some(error) = value.get("error") {
        return ParsedAccountResponse::Failure(business_failure(error));
    }
    let Some(infos) = value.get("balance_infos").and_then(Value::as_array) else {
        return ParsedAccountResponse::Failure(incompatible());
    };
    match build_balances(infos) {
        Some(balances) => ParsedAccountResponse::Success {
            balances,
            quota_windows: Vec::new(),
            // DeepSeek 响应不含平台时间戳，保持 None，绝不用本机时间顶替。
            provider_as_of: None,
        },
        None => ParsedAccountResponse::Failure(incompatible()),
    }
}

/// 每个币种一组：available（total_balance）+ grant + paid（topped_up_balance）。
fn build_balances(infos: &[Value]) -> Option<Vec<BalanceEntry>> {
    let mut out = Vec::new();
    for info in infos {
        let currency = info.get("currency").and_then(Value::as_str)?;
        if currency.is_empty() {
            return None;
        }
        let total = common::decimal_field(info.get("total_balance")?)?;
        let granted = common::decimal_field(info.get("granted_balance")?)?;
        let topped_up = common::decimal_field(info.get("topped_up_balance")?)?;
        out.push(
            BalanceEntry::new(BalanceKind::Available, total, currency)
                .ok()?
                .with_formula("total = granted + topped_up"),
        );
        out.push(BalanceEntry::new(BalanceKind::Grant, granted, currency).ok()?);
        out.push(BalanceEntry::new(BalanceKind::Paid, topped_up, currency).ok()?);
    }
    Some(out)
}

fn failure_from_status(status: u16) -> FailureSignal {
    FailureSignal {
        http_status: Some(status),
        ..Default::default()
    }
}

fn incompatible() -> FailureSignal {
    FailureSignal {
        schema_changed: true,
        ..Default::default()
    }
}

fn business_failure(error: &Value) -> FailureSignal {
    let code = common::business_code_of(error).unwrap_or_else(|| "business_error".into());
    FailureSignal {
        http_status: Some(200),
        business_code: Some(code),
        status_hint: Some(SnapshotStatus::Unavailable),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_exact_and_secret_free() {
        let spec = build_request(&RequestConfig::canonical(CANONICAL_HOST)).unwrap();
        assert_eq!(spec.method, "GET");
        assert_eq!(spec.host, CANONICAL_HOST);
        assert_eq!(spec.path, REQUEST_PATH);
        assert_eq!(spec.url(), "https://api.deepseek.com/user/balance");
        assert!(spec.headers.is_empty());
        let auth = spec.auth.as_ref().unwrap();
        assert_eq!(auth.header, "Authorization");
        assert_eq!(auth.scheme, "Bearer ");
        let placeholder = spec.auth_header_placeholder().unwrap();
        assert!(placeholder.contains("<secret-injected-by-service>"));
    }

    #[test]
    fn host_must_match_exactly_not_by_substring() {
        assert!(build_request(&RequestConfig::new("https://api.deepseek.com")).is_ok());
        // 仅“包含” api.deepseek.com 的域名必须被拒绝
        assert!(build_request(&RequestConfig::new("api.deepseek.com.evil.com")).is_err());
        assert!(build_request(&RequestConfig::new("evil-api.deepseek.com")).is_err());
        assert!(build_request(&RequestConfig::new("")).is_err());
    }

    #[test]
    fn parses_single_currency_with_components() {
        let body = r#"{"is_available":true,"balance_infos":[
            {"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Success { balances, .. } => {
                assert_eq!(balances.len(), 3);
                assert_eq!(balances[0].kind, BalanceKind::Available);
                assert_eq!(balances[0].amount, "110.00");
                assert_eq!(
                    balances[0].formula.as_deref(),
                    Some("total = granted + topped_up")
                );
                assert_eq!(balances[1].kind, BalanceKind::Grant);
                assert_eq!(balances[1].amount, "10.00");
                assert_eq!(balances[2].kind, BalanceKind::Paid);
                assert_eq!(balances[2].amount, "100.00");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn unavailable_flag_is_still_a_success() {
        let body = r#"{"is_available":false,"balance_infos":[
            {"currency":"CNY","total_balance":"0.00","granted_balance":"0.00","topped_up_balance":"0.00"}]}"#;
        assert!(matches!(
            parse_response(200, body),
            ParsedAccountResponse::Success { .. }
        ));
    }

    #[test]
    fn schema_change_does_not_zero_fill() {
        let body = r#"{"is_available":true}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Failure(signal) => {
                assert!(signal.schema_changed);
                assert_eq!(signal.status(), SnapshotStatus::Incompatible);
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
    }

    #[test]
    fn status_mapping() {
        assert_eq!(
            status_of(parse_response(401, "{}")),
            SnapshotStatus::AuthRequired
        );
        assert_eq!(
            status_of(parse_response(429, "{}")),
            SnapshotStatus::RateLimited
        );
        assert_eq!(
            status_of(parse_response(500, "{}")),
            SnapshotStatus::Unavailable
        );
    }

    fn status_of(parsed: ParsedAccountResponse) -> SnapshotStatus {
        match parsed {
            ParsedAccountResponse::Failure(signal) => signal.status(),
            other => panic!("expected failure, got {other:?}"),
        }
    }
}

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
//! 本模块自包含请求/响应类型（openrouter / novita 各自独立），便于并行开发时互不影响。

use crate::domain::decimal::decimal_add;
use crate::services::accounts::{
    BalanceEntry, BalanceKind, CredentialType, FailureSignal, QuotaWindow, SnapshotStatus,
};
use serde_json::Value;

/// 官方文档规定的精确 host（比较必须精确，禁止“包含该串”的域名）。
pub const CANONICAL_HOST: &str = "api.deepseek.com";
/// 固定路径。
pub const REQUEST_PATH: &str = "/user/balance";
/// 该接口接受的凭据类型（Bearer API Key）。
pub const CREDENTIAL_TYPE: CredentialType = CredentialType::ApiKey;
/// 余额接口超时提示（秒）；服务层可覆盖。
pub const TIMEOUT_HINT_SECONDS: u64 = 15;

/// 认证头位置：只描述“头名 + 方案前缀”。秘密值由服务层注入，绝不进入本结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPlacement {
    pub header: String,
    pub scheme: String,
}

impl AuthPlacement {
    fn bearer() -> Self {
        Self {
            header: "Authorization".into(),
            scheme: "Bearer ".into(),
        }
    }

    /// 供 UI / 日志展示的占位（不含任何秘密）。
    pub fn placeholder(&self) -> String {
        format!("{}<secret-injected-by-service>", self.scheme)
    }
}

/// 请求描述。`headers` 只含非秘密静态头；秘密位置由 `auth` 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    pub method: String,
    pub host: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub auth: Option<AuthPlacement>,
    pub timeout_hint_seconds: u64,
}

impl RequestSpec {
    pub fn url(&self) -> String {
        format!("https://{}{}", self.host, self.path)
    }

    /// `"Authorization: Bearer <secret-injected-by-service>"`；仅为形状说明，不含秘密。
    pub fn auth_header_placeholder(&self) -> Option<String> {
        self.auth
            .as_ref()
            .map(|auth| format!("{}: {}", auth.header, auth.placeholder()))
    }
}

/// 请求配置：账户侧配置的 host（可为裸域名或 URL）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestConfig {
    pub host: String,
}

impl RequestConfig {
    pub fn canonical() -> Self {
        Self {
            host: CANONICAL_HOST.into(),
        }
    }

    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestBuildError {
    #[error("deepseek host not allowed: expected {expected}, got {got}")]
    HostNotAllowed { expected: &'static str, got: String },
}

/// 构造请求。配置 host 解析出的域名必须**精确等于** [`CANONICAL_HOST`]。
pub fn build_request(cfg: &RequestConfig) -> Result<RequestSpec, RequestBuildError> {
    if extract_host(&cfg.host) != Some(CANONICAL_HOST) {
        return Err(RequestBuildError::HostNotAllowed {
            expected: CANONICAL_HOST,
            got: cfg.host.clone(),
        });
    }
    Ok(RequestSpec {
        method: "GET".into(),
        host: CANONICAL_HOST.into(),
        path: REQUEST_PATH.into(),
        headers: Vec::new(),
        auth: Some(AuthPlacement::bearer()),
        timeout_hint_seconds: TIMEOUT_HINT_SECONDS,
    })
}

/// 解析结果：成功返回标准化余额/窗口 + 平台数据时间；失败返回 [`FailureSignal`] 语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedAccountResponse {
    Success {
        balances: Vec<BalanceEntry>,
        quota_windows: Vec<QuotaWindow>,
        provider_as_of: Option<String>,
    },
    Failure(FailureSignal),
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
        let total = decimal_field(info.get("total_balance")?)?;
        let granted = decimal_field(info.get("granted_balance")?)?;
        let topped_up = decimal_field(info.get("topped_up_balance")?)?;
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
    let code = business_code_of(error).unwrap_or_else(|| "business_error".into());
    FailureSignal {
        http_status: Some(200),
        business_code: Some(code),
        status_hint: Some(SnapshotStatus::Unavailable),
        ..Default::default()
    }
}

/// 金额字段：官方为字符串小数，同时兼容 JSON 数字（整数精确、浮点避免科学计数）。
fn decimal_field(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => valid_decimal(s),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                valid_decimal(&i.to_string())
            } else if let Some(u) = n.as_u64() {
                valid_decimal(&u.to_string())
            } else {
                let f = n.as_f64()?;
                if !f.is_finite() {
                    return None;
                }
                valid_decimal(&format!("{f}"))
            }
        }
        _ => None,
    }
}

fn valid_decimal(value: &str) -> Option<String> {
    decimal_add(value, "0").map(|_| value.to_string())
}

fn business_code_of(error: &Value) -> Option<String> {
    error
        .get("code")
        .and_then(value_code)
        .or_else(|| error.get("type").and_then(value_code))
}

fn value_code(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 从裸域名或 URL 中取出 host 段，供**精确**比较（不是子串匹配）。
fn extract_host(input: &str) -> Option<&str> {
    let after_scheme = input.split_once("://").map(|(_, rest)| rest).unwrap_or(input);
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_exact_and_secret_free() {
        let spec = build_request(&RequestConfig::canonical()).unwrap();
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
        assert_eq!(status_of(parse_response(401, "{}")), SnapshotStatus::AuthRequired);
        assert_eq!(status_of(parse_response(429, "{}")), SnapshotStatus::RateLimited);
        assert_eq!(status_of(parse_response(500, "{}")), SnapshotStatus::Unavailable);
    }

    fn status_of(parsed: ParsedAccountResponse) -> SnapshotStatus {
        match parsed {
            ParsedAccountResponse::Failure(signal) => signal.status(),
            other => panic!("expected failure, got {other:?}"),
        }
    }
}

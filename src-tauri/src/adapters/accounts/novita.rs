//! B04 Novita 余额适配器（纯请求构造 + 纯响应解析；不触网、不读凭据）。
//!
//! 证据：docs/research/evidence/accounts-b01-b07-candidates.md「B04 Novita 余额」（E1）；
//! 规范：docs/specs/ACCOUNT_ADAPTERS.md 第 1/5/6/7 节、docs/specs/DATA_CONTRACTS.md 第 7 节。
//!
//! 接口：`GET https://api.novita.ai/openapi/v1/billing/balance/detail`，
//! `Authorization: Bearer <API_KEY>`，且请求头 `Content-Type: application/json`（文档标 required）。
//! 响应字段 `availableBalance` / `cashBalance` / `creditLimit` / `pendingCharges` /
//! `outstandingInvoices`，原值单位 **1/10000 USD**（文档：`10000 = $1.00`）。
//! 换算必须**整数除以 10000**（十进制字符串移位），不使用二进制浮点；负余额保留真实值。
//!
//! 本模块自包含请求/响应类型（deepseek / openrouter 各自独立），便于并行开发时互不影响。

use crate::services::accounts::{
    BalanceEntry, BalanceKind, CredentialType, FailureSignal, QuotaWindow, SnapshotStatus,
};
use serde_json::{Map, Value};

/// 官方文档规定的精确 host（比较必须精确，禁止“包含该串”的域名）。
pub const CANONICAL_HOST: &str = "api.novita.ai";
/// 当前官方路径（不采用 cc-switch 旧线索 `/v3/user/balance`）。
pub const REQUEST_PATH: &str = "/openapi/v1/billing/balance/detail";
/// 该接口接受的凭据类型（Bearer API Key）。
pub const CREDENTIAL_TYPE: CredentialType = CredentialType::ApiKey;
/// 余额接口超时提示（秒）；服务层可覆盖。
pub const TIMEOUT_HINT_SECONDS: u64 = 15;
/// 原值单位：1 USD = 10000 个计费单位。
const UNITS_PER_USD: usize = 10_000;

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
    #[error("novita host not allowed: expected {expected}, got {got}")]
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
        // 文档标 required 的静态头（不含秘密）。
        headers: vec![("Content-Type".into(), "application/json".into())],
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

/// 解析响应。非 200 交给 [`FailureSignal`] 默认映射；HTTP 200 业务错误与 schema 变化按失败处理。
pub fn parse_response(status: u16, body: &str) -> ParsedAccountResponse {
    if status != 200 {
        return ParsedAccountResponse::Failure(failure_from_status(status));
    }
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    };
    let Some(obj) = value.as_object() else {
        return ParsedAccountResponse::Failure(incompatible());
    };
    // HTTP 200 + 业务错误体（缺 availableBalance，但带非 0 code / error 字段）。
    if !obj.contains_key("availableBalance") {
        return match business_code_of(obj) {
            Some(code) => ParsedAccountResponse::Failure(FailureSignal {
                http_status: Some(200),
                business_code: Some(code),
                status_hint: Some(SnapshotStatus::Unavailable),
                ..Default::default()
            }),
            None => ParsedAccountResponse::Failure(incompatible()),
        };
    }
    match build_balances(obj) {
        Some(balances) => ParsedAccountResponse::Success {
            balances,
            quota_windows: Vec::new(),
            provider_as_of: None,
        },
        None => ParsedAccountResponse::Failure(incompatible()),
    }
}

/// 五个字段 → 余额行；金额由 1/10000 USD 精确换算为 USD 十进制字符串。
///
/// `BalanceKind` 没有“负债/欠款”种类：`creditLimit` / `pendingCharges` /
/// `outstandingInvoices` 归入 `credit` 并在 formula 标注其“非现金”语义，UI 不得当作可用现金。
fn build_balances(obj: &Map<String, Value>) -> Option<Vec<BalanceEntry>> {
    let available = usd_field(obj, "availableBalance")?;
    let cash = usd_field(obj, "cashBalance")?;
    let credit_limit = usd_field(obj, "creditLimit")?;
    let pending = usd_field(obj, "pendingCharges")?;
    let invoices = usd_field(obj, "outstandingInvoices")?;
    let mut out = Vec::new();
    out.push(
        BalanceEntry::new(BalanceKind::Available, available, "USD")
            .ok()?
            .with_formula("USD = availableBalance / 10000 (1/10000 USD)"),
    );
    out.push(
        BalanceEntry::new(BalanceKind::Paid, cash, "USD")
            .ok()?
            .with_formula("USD = cashBalance / 10000 (充值剩余, 1/10000 USD)"),
    );
    out.push(
        BalanceEntry::new(BalanceKind::Credit, credit_limit, "USD")
            .ok()?
            .with_formula("USD = creditLimit / 10000 (授信上限, 非现金)"),
    );
    out.push(
        BalanceEntry::new(BalanceKind::Credit, pending, "USD")
            .ok()?
            .with_formula("USD = pendingCharges / 10000 (待结算, 非现金)"),
    );
    out.push(
        BalanceEntry::new(BalanceKind::Credit, invoices, "USD")
            .ok()?
            .with_formula("USD = outstandingInvoices / 10000 (欠款, 非现金)"),
    );
    Some(out)
}

fn usd_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    let raw = raw_integer(obj.get(key)?)?;
    units_to_usd(&raw)
}

/// 取整数原值（官方为十进制整数字符串，同时兼容 JSON 整数）。
fn raw_integer(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n
            .as_i64()
            .map(|i| i.to_string())
            .or_else(|| n.as_u64().map(|u| u.to_string())),
        _ => None,
    }
}

/// 1/10000 USD → USD 十进制字符串（整数运算/字符串移位，无浮点）。非法输入返回 None。
fn units_to_usd(raw: &str) -> Option<String> {
    let (neg, rest) = if let Some(r) = raw.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = raw.strip_prefix('+') {
        (false, r)
    } else {
        (false, raw)
    };
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let scale = UNITS_PER_USD.ilog10() as usize; // 4
    let (int_part, frac_part) = if rest.len() > scale {
        rest.split_at(rest.len() - scale)
    } else {
        ("0", "")
    };
    let frac = if frac_part.is_empty() {
        // 左侧补零到 scale 位
        let mut padded = String::from("0".repeat(scale - rest.len()));
        padded.push_str(rest);
        padded
    } else {
        frac_part.to_string()
    };
    let frac_trimmed = frac.trim_end_matches('0');
    let int_trimmed = int_part.trim_start_matches('0');
    let int_out = if int_trimmed.is_empty() { "0" } else { int_trimmed };
    let body = if frac_trimmed.is_empty() {
        int_out.to_string()
    } else {
        format!("{int_out}.{frac_trimmed}")
    };
    if neg && body != "0" {
        Some(format!("-{body}"))
    } else {
        Some(body)
    }
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

/// Novita 错误体形如 `{"code": <非 0>, "message": ...}`；`code=0` 或缺失不算业务错误。
fn business_code_of(obj: &Map<String, Value>) -> Option<String> {
    match obj.get("code") {
        Some(Value::Number(n)) => {
            let value = n.as_i64().unwrap_or(-1);
            (value != 0).then(|| value.to_string())
        }
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => obj.contains_key("error").then(|| "business_error".to_string()),
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
    fn request_has_required_content_type() {
        let spec = build_request(&RequestConfig::canonical()).unwrap();
        assert_eq!(spec.url(), "https://api.novita.ai/openapi/v1/billing/balance/detail");
        assert!(spec
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        assert!(build_request(&RequestConfig::new("api.novita.ai.evil.com")).is_err());
    }

    #[test]
    fn units_convert_exactly_without_float() {
        assert_eq!(units_to_usd("1000000").as_deref(), Some("100"));
        assert_eq!(units_to_usd("800000").as_deref(), Some("80"));
        assert_eq!(units_to_usd("12345").as_deref(), Some("1.2345"));
        assert_eq!(units_to_usd("1").as_deref(), Some("0.0001"));
        assert_eq!(units_to_usd("0").as_deref(), Some("0"));
        assert_eq!(units_to_usd("-1").as_deref(), Some("-0.0001"));
        assert_eq!(units_to_usd("-5000").as_deref(), Some("-0.5"));
        assert_eq!(units_to_usd("-0").as_deref(), Some("0"));
        assert!(units_to_usd("1.5").is_none());
        assert!(units_to_usd("abc").is_none());
    }

    #[test]
    fn parses_balance_detail() {
        let body = r#"{"availableBalance":"1000000","cashBalance":"800000",
            "creditLimit":"200000","pendingCharges":"0","outstandingInvoices":"0"}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Success { balances, .. } => {
                assert_eq!(balances.len(), 5);
                assert_eq!(balances[0].kind, BalanceKind::Available);
                assert_eq!(balances[0].amount, "100");
                assert_eq!(balances[1].kind, BalanceKind::Paid);
                assert_eq!(balances[1].amount, "80");
                assert_eq!(balances[2].amount, "20");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn negative_balance_is_preserved() {
        let body = r#"{"availableBalance":"-5000","cashBalance":"0",
            "creditLimit":"0","pendingCharges":"0","outstandingInvoices":"0"}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Success { balances, .. } => {
                assert_eq!(balances[0].amount, "-0.5");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn schema_change_does_not_zero_fill() {
        let body = r#"{"availableBalance":"1000000"}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Failure(signal) => {
                assert!(signal.schema_changed);
                assert_eq!(signal.status(), SnapshotStatus::Incompatible);
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
    }

    #[test]
    fn business_error_on_http_200_is_a_failure() {
        let body = r#"{"code":40101,"message":"invalid api key"}"#;
        match parse_response(200, body) {
            ParsedAccountResponse::Failure(signal) => {
                assert_eq!(signal.business_code.as_deref(), Some("40101"));
                assert!(signal.is_failure());
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }
}

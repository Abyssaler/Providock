//! B02 / B03 OpenRouter 适配器（纯请求构造 + 纯响应解析；不触网、不读凭据）。
//!
//! 证据：docs/research/evidence/accounts-b01-b07-candidates.md「B02 / B03」（E1）；
//! 规范：docs/specs/ACCOUNT_ADAPTERS.md 第 1/5/6/7 节、docs/specs/DATA_CONTRACTS.md 第 7/8 节。
//!
//! - B02 `GET https://openrouter.ai/api/v1/key`（Bearer 推理 Key）：当前 Key 的用量/预算，
//!   单位 USD，`scope=key`。这是 **Key/成员预算，不是账户现金余额**，故 balances 为空，
//!   只产出 quota_windows。`limit` / `limit_remaining` 未设限时可能为 null（合法，不算 schema 变化）。
//! - B03 `GET https://openrouter.ai/api/v1/credits`（**管理密钥**）：账户 `total_credits` /
//!   `total_usage`，可用额按官方语义 `available = total_credits - total_usage`（精确十进制相减）。
//!   普通推理 Key 会 403 → auth_required。
//!
//! 请求/响应类型与十进制工具统一取自 [`crate::adapters::accounts::common`]。

use crate::adapters::accounts::common::{self, AuthPlacement, RequestSpec};
use crate::domain::Quality;
use crate::services::accounts::{
    BalanceEntry, BalanceKind, CredentialType, FailureSignal, PeriodKind, QuotaUnit, QuotaWindow,
    SnapshotStatus,
};
use serde_json::{Map, Value};

// 公共类型在本模块的再导出：适配器不再各自定义，旧的引用路径保持不变。
pub use crate::adapters::accounts::common::ParsedAccountResponse;

/// 官方文档规定的精确 host（比较必须精确，禁止“包含该串”的域名）。
pub const CANONICAL_HOST: &str = "openrouter.ai";
/// 余额接口超时提示（秒）；服务层可覆盖。
pub const TIMEOUT_HINT_SECONDS: u64 = common::DEFAULT_TIMEOUT_SECONDS;

/// 同一 host 下的两个端点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// B02 当前推理 Key 的用量/预算。
    CurrentKey,
    /// B03 账户 Credits（需管理密钥）。
    Credits,
}

impl Endpoint {
    pub fn path(self) -> &'static str {
        match self {
            Endpoint::CurrentKey => "/api/v1/key",
            Endpoint::Credits => "/api/v1/credits",
        }
    }

    /// 该端点要求的凭据类型：当前 Key 用普通推理 Key；Credits 必须用管理密钥（普通 Key 会 403）。
    pub fn credential_type(self) -> CredentialType {
        match self {
            Endpoint::CurrentKey => CredentialType::ApiKey,
            Endpoint::Credits => CredentialType::ManagementKey,
        }
    }
}

/// 请求配置：账户侧配置的 host（可为裸域名或 URL）+ 端点选择。
///
/// host 语义与 [`common::RequestConfig`] 一致，这里额外携带 OpenRouter 的两个端点之一。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestConfig {
    pub host: String,
    pub endpoint: Endpoint,
}

impl RequestConfig {
    pub fn canonical(endpoint: Endpoint) -> Self {
        Self {
            host: CANONICAL_HOST.into(),
            endpoint,
        }
    }

    pub fn new(host: impl Into<String>, endpoint: Endpoint) -> Self {
        Self {
            host: host.into(),
            endpoint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestBuildError {
    #[error("openrouter host not allowed: expected {expected}, got {got}")]
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
    Ok(RequestSpec::https(CANONICAL_HOST, cfg.endpoint.path()).with_auth(AuthPlacement::bearer()))
}

/// 分发到对应端点的解析器。
pub fn parse_response(endpoint: Endpoint, status: u16, body: &str) -> ParsedAccountResponse {
    match endpoint {
        Endpoint::CurrentKey => parse_current_key(status, body),
        Endpoint::Credits => parse_credits(status, body),
    }
}

/// B02：当前 Key 的用量/预算 → quota_windows（scope=key）。**不产出现金余额。**
pub fn parse_current_key(status: u16, body: &str) -> ParsedAccountResponse {
    if status != 200 {
        return ParsedAccountResponse::Failure(failure_from_status(status));
    }
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    };
    if let Some(error) = value.get("error") {
        return ParsedAccountResponse::Failure(business_failure(error));
    }
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return ParsedAccountResponse::Failure(incompatible());
    };
    match build_key_windows(data) {
        // 真实响应至少含 usage 一类字段；一个窗口都产不出说明字段整体变化 → incompatible，不补零。
        Ok(windows) if windows.is_empty() => ParsedAccountResponse::Failure(incompatible()),
        Ok(windows) => ParsedAccountResponse::Success {
            balances: Vec::new(),
            quota_windows: windows,
            provider_as_of: None,
        },
        Err(()) => ParsedAccountResponse::Failure(incompatible()),
    }
}

/// B03：账户 Credits。`available = total_credits - total_usage`（精确十进制）。
pub fn parse_credits(status: u16, body: &str) -> ParsedAccountResponse {
    if status != 200 {
        return ParsedAccountResponse::Failure(failure_from_status(status));
    }
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    };
    if let Some(error) = value.get("error") {
        return ParsedAccountResponse::Failure(business_failure(error));
    }
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return ParsedAccountResponse::Failure(incompatible());
    };
    let total = match common::opt_decimal(data, "total_credits") {
        Ok(Some(value)) => value,
        _ => return ParsedAccountResponse::Failure(incompatible()),
    };
    let usage = match common::opt_decimal(data, "total_usage") {
        Ok(Some(value)) => value,
        _ => return ParsedAccountResponse::Failure(incompatible()),
    };
    let available = match crate::domain::decimal::decimal_add(&total, &negate(&usage)) {
        Some(value) => value,
        None => return ParsedAccountResponse::Failure(incompatible()),
    };
    let mut balances = Vec::new();
    match BalanceEntry::new(BalanceKind::Available, available, "USD") {
        Ok(entry) => balances.push(
            entry
                .with_formula("available = total_credits - total_usage")
                .with_quality(Quality::Derived),
        ),
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    }
    match BalanceEntry::new(BalanceKind::Credit, total, "USD") {
        Ok(entry) => balances.push(entry.with_formula("total_credits (purchased)")),
        Err(_) => return ParsedAccountResponse::Failure(incompatible()),
    }
    ParsedAccountResponse::Success {
        balances,
        quota_windows: Vec::new(),
        provider_as_of: None,
    }
}

/// B02 顶层字段 → 窗口。没有值的字段不造空窗口；类型错误（非 number/null）视为 schema 变化。
fn build_key_windows(data: &Map<String, Value>) -> Result<Vec<QuotaWindow>, ()> {
    let mut windows = Vec::new();

    let usage = common::opt_decimal(data, "usage")?;
    let limit = common::opt_decimal(data, "limit")?;
    let remaining = common::opt_decimal(data, "limit_remaining")?;
    let reset = common::opt_string(data, "limit_reset")?;
    let (period, duration) = reset_period(reset.as_deref());
    if let Some(window) = window_if_any(
        "openrouter:key:spend_limit:usd",
        QuotaUnit::Currency,
        "Key 消费上限 (USD)",
        usage.clone(),
        limit,
        remaining,
        period,
        duration,
    ) {
        windows.push(window);
    }

    if let Some(window) = window_if_any(
        "openrouter:key:usage:total:usd",
        QuotaUnit::Currency,
        "Key 累计用量 (USD)",
        usage,
        None,
        None,
        None,
        None,
    ) {
        windows.push(window);
    }

    for (key, id, label, duration) in [
        (
            "usage_daily",
            "openrouter:key:usage:daily:usd",
            "Key 当日用量 (USD)",
            Some(86_400u64),
        ),
        (
            "usage_weekly",
            "openrouter:key:usage:weekly:usd",
            "Key 本周用量 (USD)",
            Some(604_800u64),
        ),
        (
            "usage_monthly",
            "openrouter:key:usage:monthly:usd",
            "Key 本月用量 (USD)",
            None,
        ),
    ] {
        let used = common::opt_decimal(data, key)?;
        if let Some(window) = window_if_any(
            id,
            QuotaUnit::Currency,
            label,
            used,
            None,
            None,
            Some(PeriodKind::Calendar),
            duration,
        ) {
            windows.push(window);
        }
    }

    // free_model_daily_requests: { limit, remaining, used } → 请求数单位窗口。
    match data.get("free_model_daily_requests") {
        None | Some(Value::Null) => {}
        Some(Value::Object(free)) => {
            let used = common::opt_decimal(free, "used")?;
            let limit = common::opt_decimal(free, "limit")?;
            let remaining = common::opt_decimal(free, "remaining")?;
            if let Some(window) = window_if_any(
                "openrouter:key:free_model_daily_requests:requests",
                QuotaUnit::Requests,
                "免费模型每日请求数",
                used,
                limit,
                remaining,
                Some(PeriodKind::Calendar),
                Some(86_400),
            ) {
                windows.push(window);
            }
        }
        Some(_) => return Err(()),
    }

    Ok(windows)
}

/// 仅当 used / limit / remaining 至少一项已知时才产出窗口（不造全空窗口）。
// 参数多于 7 个：每个字段都是独立的窗口属性，用结构体会让调用点更笨重。
#[allow(clippy::too_many_arguments)]
fn window_if_any(
    window_id: &str,
    unit: QuotaUnit,
    label: &str,
    used: Option<String>,
    limit: Option<String>,
    remaining: Option<String>,
    period: Option<PeriodKind>,
    duration_seconds: Option<u64>,
) -> Option<QuotaWindow> {
    if used.is_none() && limit.is_none() && remaining.is_none() {
        return None;
    }
    let mut window = QuotaWindow::new(window_id, unit);
    window.scope = Some("key".into());
    window.label = Some(label.into());
    window.period_kind = period;
    window.duration_seconds = duration_seconds;
    window.used = used;
    window.limit = limit;
    window.remaining = remaining;
    Some(window)
}

/// `limit_reset` 取值（daily/weekly/monthly/null）→ 周期类型与已知时长。
fn reset_period(reset: Option<&str>) -> (Option<PeriodKind>, Option<u64>) {
    match reset {
        Some("daily") => (Some(PeriodKind::Calendar), Some(86_400)),
        Some("weekly") => (Some(PeriodKind::Calendar), Some(604_800)),
        Some("monthly") => (Some(PeriodKind::Calendar), None),
        _ => (None, None),
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

/// HTTP 200 业务错误体（`{"error":{"code":401,...}}`）。
fn business_failure(error: &Value) -> FailureSignal {
    let code = common::business_code_of(error).unwrap_or_else(|| "business_error".into());
    let hint = match code.as_str() {
        "401" | "403" => SnapshotStatus::AuthRequired,
        "429" => SnapshotStatus::RateLimited,
        _ => SnapshotStatus::Unavailable,
    };
    FailureSignal {
        http_status: Some(200),
        business_code: Some(code),
        status_hint: Some(hint),
        ..Default::default()
    }
}

/// 取相反数（保留十进制字符串形态，交给 `decimal_add` 精确相加）。
fn negate(value: &str) -> String {
    match value.strip_prefix('-') {
        Some(rest) => rest.to_string(),
        None => format!("-{value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths_and_host() {
        let key = build_request(&RequestConfig::canonical(Endpoint::CurrentKey)).unwrap();
        assert_eq!(key.url(), "https://openrouter.ai/api/v1/key");
        let credits = build_request(&RequestConfig::canonical(Endpoint::Credits)).unwrap();
        assert_eq!(credits.url(), "https://openrouter.ai/api/v1/credits");
        assert!(build_request(&RequestConfig::new(
            "openrouter.ai.evil.com",
            Endpoint::CurrentKey
        ))
        .is_err());
    }

    #[test]
    fn current_key_yields_no_cash_balance() {
        let body = r#"{"data":{"limit":100,"limit_remaining":74.25,"limit_reset":"monthly",
            "usage":25.75,"usage_daily":1.5,"usage_weekly":9.5,"usage_monthly":25.75}}"#;
        match parse_current_key(200, body) {
            ParsedAccountResponse::Success {
                balances,
                quota_windows,
                ..
            } => {
                assert!(balances.is_empty(), "Key 预算不是现金余额");
                let ids: Vec<&str> = quota_windows.iter().map(|w| w.window_id.as_str()).collect();
                assert!(ids.contains(&"openrouter:key:spend_limit:usd"));
                assert!(ids.contains(&"openrouter:key:usage:daily:usd"));
                let limit = quota_windows
                    .iter()
                    .find(|w| w.window_id == "openrouter:key:spend_limit:usd")
                    .unwrap();
                assert_eq!(limit.scope.as_deref(), Some("key"));
                assert_eq!(limit.used.as_deref(), Some("25.75"));
                assert_eq!(limit.limit.as_deref(), Some("100"));
                assert_eq!(limit.remaining.as_deref(), Some("74.25"));
                assert_eq!(limit.period_kind, Some(PeriodKind::Calendar));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn null_limit_is_legal_not_schema_change() {
        let body = r#"{"data":{"limit":null,"limit_remaining":null,"usage":3.5}}"#;
        match parse_current_key(200, body) {
            ParsedAccountResponse::Success { quota_windows, .. } => {
                assert!(quota_windows
                    .iter()
                    .any(|w| w.window_id == "openrouter:key:usage:total:usd"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn empty_key_data_is_schema_change() {
        match parse_current_key(200, r#"{"data":{}}"#) {
            ParsedAccountResponse::Failure(signal) => {
                assert!(signal.schema_changed);
                assert_eq!(signal.status(), SnapshotStatus::Incompatible);
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
    }

    #[test]
    fn credits_available_is_exact_difference() {
        let body = r#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#;
        match parse_credits(200, body) {
            ParsedAccountResponse::Success { balances, .. } => {
                assert_eq!(balances[0].kind, BalanceKind::Available);
                assert_eq!(balances[0].amount, "74.75");
                assert_eq!(balances[0].quality, Quality::Derived);
                assert_eq!(balances[1].kind, BalanceKind::Credit);
                assert_eq!(balances[1].amount, "100.5");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn credits_forbidden_maps_to_auth_required() {
        let parsed = parse_credits(
            403,
            r#"{"error":{"code":403,"message":"Only management keys"}}"#,
        );
        match parsed {
            ParsedAccountResponse::Failure(signal) => {
                assert_eq!(signal.status(), SnapshotStatus::AuthRequired);
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn credits_schema_change_does_not_default_to_zero() {
        let body = r#"{"data":{"total_credits":10}}"#;
        match parse_credits(200, body) {
            ParsedAccountResponse::Failure(signal) => {
                assert!(signal.schema_changed);
                assert_eq!(signal.status(), SnapshotStatus::Incompatible);
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
    }
}

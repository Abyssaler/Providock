//! 账户适配器公共契约类型与纯工具。
//!
//! B01–B07 适配器原先各自复制了 `RequestSpec` / `ParsedAccountResponse` /
//! `AuthPlacement` / `RequestConfig` 以及十进制、比例工具。这里收口为唯一实现，
//! 适配器只引用，不再重复定义（行为不变）。
//!
//! 边界（ARCHITECTURE 第 4 节、ACCOUNT_ADAPTERS 第 1/7 节、SECURITY_PRIVACY 第 4 节）：
//! - 只有纯数据与纯函数；不触网、不读凭据、不引用 HTTP 客户端。
//! - `RequestSpec` 只描述 method / scheme / 精确 host / path / 非秘密头与认证**位置**；
//!   秘密由服务层按 [`AuthPlacement`] 注入，绝不进入任何适配器结构。
//! - host 一律精确比较，禁止“包含子串”式域名判定。

use crate::domain::decimal::decimal_add;
use crate::services::accounts::{BalanceEntry, FailureSignal, QuotaWindow};
use serde_json::{Map, Value};

/// 账户余额/额度接口的默认超时提示（秒）；服务层可覆盖。
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 15;

// ---------------------------------------------------------------------------
// 认证位置
// ---------------------------------------------------------------------------

/// 认证头位置：只描述“头名 + 方案前缀”，不含秘密。秘密由服务层注入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPlacement {
    pub header: String,
    pub scheme: String,
}

impl AuthPlacement {
    pub fn new(header: impl Into<String>, scheme: impl Into<String>) -> Self {
        Self {
            header: header.into(),
            scheme: scheme.into(),
        }
    }

    /// `Authorization: Bearer <secret>`。
    pub fn bearer() -> Self {
        Self::new("Authorization", "Bearer ")
    }

    /// `Authorization: <secret>`：GLM 等要求原 token，**不加** `Bearer` 前缀。
    pub fn raw_token() -> Self {
        Self::new("Authorization", "")
    }

    /// 认证由对端持有（如 Codex App Server）：适配器不注入任何头。
    pub fn server_held() -> Self {
        Self::new("", "")
    }

    pub fn is_server_held(&self) -> bool {
        self.header.is_empty() && self.scheme.is_empty()
    }

    /// 供 UI / 日志展示的占位（不含任何秘密）。
    pub fn placeholder(&self) -> String {
        format!("{}<secret-injected-by-service>", self.scheme)
    }

    /// 用真实秘密构造头值；调用方负责尽快 drop 秘密。
    pub fn header_value(&self, secret: &str) -> String {
        format!("{}{secret}", self.scheme)
    }
}

// ---------------------------------------------------------------------------
// 请求规格
// ---------------------------------------------------------------------------

/// 一次只读账户查询的请求描述。
///
/// `scheme` 外网固定 `https`；只有 Kimi 本机 Server 的 loopback 允许 `http`。
/// `host` 是精确 host（含端口），白名单比较必须精确相等。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    pub method: String,
    pub scheme: String,
    pub host: String,
    /// 路径，可含查询串。
    pub path: String,
    /// 非秘密静态头；秘密位置由 `auth` 描述。
    pub headers: Vec<(String, String)>,
    pub auth: Option<AuthPlacement>,
    pub timeout_hint_seconds: u64,
}

impl RequestSpec {
    /// 外网 HTTPS GET（最常用形态）。
    pub fn https(host: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: "GET".into(),
            scheme: "https".into(),
            host: host.into(),
            path: path.into(),
            headers: Vec::new(),
            auth: None,
            timeout_hint_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }

    pub fn with_auth(mut self, auth: AuthPlacement) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    pub fn url(&self) -> String {
        format!("{}://{}{}", self.scheme, self.host, self.path)
    }

    /// `"Authorization: Bearer <secret-injected-by-service>"`；仅为形状说明，不含秘密。
    pub fn auth_header_placeholder(&self) -> Option<String> {
        self.auth
            .as_ref()
            .map(|auth| format!("{}: {}", auth.header, auth.placeholder()))
    }
}

/// 请求配置：账户侧配置的 host（可为裸域名或 URL）。适配器据此精确校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestConfig {
    pub host: String,
}

impl RequestConfig {
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into() }
    }

    /// 语义化别名：使用某个规范 host。
    pub fn canonical(host: impl Into<String>) -> Self {
        Self::new(host)
    }
}

// ---------------------------------------------------------------------------
// 解析结果
// ---------------------------------------------------------------------------

/// HTTP 账户适配器的统一解析结果：成功返回标准化余额/窗口 + 平台数据时间；
/// 失败返回 [`FailureSignal`] 语义（HTTP 状态、业务码、schema 变化、能力缺失）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedAccountResponse {
    Success {
        balances: Vec<BalanceEntry>,
        quota_windows: Vec<QuotaWindow>,
        provider_as_of: Option<String>,
    },
    Failure(FailureSignal),
}

impl ParsedAccountResponse {
    pub fn success(balances: Vec<BalanceEntry>, quota_windows: Vec<QuotaWindow>) -> Self {
        Self::Success {
            balances,
            quota_windows,
            provider_as_of: None,
        }
    }

    pub fn failure(signal: FailureSignal) -> Self {
        Self::Failure(signal)
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

// ---------------------------------------------------------------------------
// host 解析与校验
// ---------------------------------------------------------------------------

/// 从裸域名或 URL 中取出 host 段，供**精确**比较（不是子串匹配）。
pub fn extract_host(input: &str) -> Option<&str> {
    let after_scheme = input
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(input);
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    (!host.is_empty()).then_some(host)
}

/// `host[:port]`，支持 `[::1]:port` 形态。
pub fn split_host_port(authority: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some(p.parse::<u16>().ok()?),
            Some(_) => return None,
            None => None,
        };
        return Some((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            Some((host.to_owned(), Some(p.parse::<u16>().ok()?)))
        }
        Some(_) => None,
        None => Some((authority.to_owned(), None)),
    }
}

// ---------------------------------------------------------------------------
// JSON → 十进制 / 整数
// ---------------------------------------------------------------------------

/// 金额字段：官方为字符串小数，同时兼容 JSON 数字（整数精确、浮点避免科学计数）。
pub fn decimal_field(value: &Value) -> Option<String> {
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

/// 校验十进制字符串（可选负号；整数或带小数）。
pub fn valid_decimal(value: &str) -> Option<String> {
    decimal_add(value, "0").map(|_| value.to_string())
}

/// 对象字段 → 可选十进制：缺失/null 为 `None`；类型错误为 `Err(())`（schema 变化）。
// `Err(())` 是刻意的三态信号（缺失 / null / 类型错误），调用方据此判定 schema 变化。
#[allow(clippy::result_unit_err)]
pub fn opt_decimal(data: &Map<String, Value>, key: &str) -> Result<Option<String>, ()> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => decimal_field(value).ok_or(()).map(Some),
    }
}

/// 对象字段 → 可选字符串：缺失/null 为 `None`；非字符串为 `Err(())`。
#[allow(clippy::result_unit_err)]
pub fn opt_string(data: &Map<String, Value>, key: &str) -> Result<Option<String>, ()> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(()),
    }
}

/// JSON 数字或十进制字符串 → 十进制字符串（保留源形态，不做浮点运算）。
pub fn json_decimal(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => {
            let text = text.trim();
            if is_decimal(text) {
                Some(text.to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn is_decimal(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let mut parts = body.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    if int.is_empty() && frac.is_empty() {
        return false;
    }
    int.chars().all(|c| c.is_ascii_digit()) && frac.chars().all(|c| c.is_ascii_digit())
}

pub fn json_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// HTTP 200 业务错误体中的错误码：`code`（字符串/数字）优先，其次 `type`。
pub fn business_code_of(error: &Value) -> Option<String> {
    error
        .get("code")
        .and_then(value_code)
        .or_else(|| error.get("type").and_then(value_code))
}

pub fn value_code(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 十进制移位与比例
// ---------------------------------------------------------------------------

/// 十进制字符串乘以 10^places（`places>0`）或除以 10^places（`places<0`），精确、去尾零。
///
/// `"25"` 乘 -2 → `"0.25"`；`"1234"` 乘 -2 → `"12.34"`；`"43.5"` 乘 -2 → `"0.435"`。
pub fn shift_decimal_by(value: &str, places: i32) -> Option<String> {
    let (neg, body) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    if body.is_empty() {
        return None;
    }
    let mut parts = body.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next().unwrap_or("");
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let all = format!("{int_part}{frac_part}");
    let point = int_part.len() as i64 + places as i64;
    let (int_digits, frac_digits) = if point <= 0 {
        (
            "0".to_owned(),
            format!("{}{all}", "0".repeat((-point) as usize)),
        )
    } else {
        let point = point as usize;
        if point >= all.len() {
            (
                format!("{all}{}", "0".repeat(point - all.len())),
                String::new(),
            )
        } else {
            (all[..point].to_owned(), all[point..].to_owned())
        }
    };
    let int_norm = match int_digits.trim_start_matches('0') {
        "" => "0",
        rest => rest,
    };
    let frac_norm = frac_digits.trim_end_matches('0');
    let mut out = if frac_norm.is_empty() {
        int_norm.to_owned()
    } else {
        format!("{int_norm}.{frac_norm}")
    };
    if out == "0" {
        return Some(out);
    }
    if neg {
        out.insert(0, '-');
    }
    Some(out)
}

/// 比例归一：>1 时 `used_ratio` 封顶 1、`remaining_ratio` 0，原值进 `raw_ratio`。
pub fn normalize_ratio(ratio: &str) -> (String, Option<String>, Option<String>, Option<bool>) {
    if is_gt_one(ratio) {
        (
            "1".to_owned(),
            Some("0".to_owned()),
            Some(ratio.to_owned()),
            Some(true),
        )
    } else {
        (ratio.to_owned(), one_minus(ratio), None, None)
    }
}

/// `1 - ratio`（比例之间可换算；绝对量不可）。
pub fn one_minus(ratio: &str) -> Option<String> {
    let negated = match ratio.strip_prefix('-') {
        Some(rest) => rest.to_owned(),
        None => format!("-{ratio}"),
    };
    decimal_add("1", &negated)
}

/// 十进制字符串是否 > 1。
pub fn is_gt_one(value: &str) -> bool {
    let body = value.strip_prefix('-').unwrap_or(value);
    let mut parts = body.splitn(2, '.');
    let int = parts.next().unwrap_or("0");
    let frac = parts.next().unwrap_or("");
    match int.trim_start_matches('0') {
        "" => false,
        "1" => frac.chars().any(|c| c != '0'),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_placements_carry_no_secret() {
        assert_eq!(
            AuthPlacement::bearer().placeholder(),
            "Bearer <secret-injected-by-service>"
        );
        assert_eq!(AuthPlacement::raw_token().header_value("tok"), "tok");
        assert_eq!(AuthPlacement::bearer().header_value("tok"), "Bearer tok");
        assert!(AuthPlacement::server_held().is_server_held());
        assert_eq!(
            AuthPlacement::server_held().placeholder(),
            "<secret-injected-by-service>"
        );
    }

    #[test]
    fn request_spec_url_is_exact() {
        let spec = RequestSpec::https("api.deepseek.com", "/user/balance")
            .with_auth(AuthPlacement::bearer());
        assert_eq!(spec.url(), "https://api.deepseek.com/user/balance");
        assert_eq!(spec.method, "GET");
        assert!(spec
            .auth_header_placeholder()
            .unwrap()
            .contains("<secret-injected-by-service>"));
    }

    #[test]
    fn extract_host_is_exact_not_substring() {
        assert_eq!(
            extract_host("https://api.deepseek.com"),
            Some("api.deepseek.com")
        );
        assert_eq!(extract_host("api.deepseek.com"), Some("api.deepseek.com"));
        assert_eq!(extract_host("https://user@host:8443/x"), Some("host:8443"));
        assert_ne!(
            extract_host("api.deepseek.com.evil.com"),
            Some("api.deepseek.com")
        );
        assert_eq!(extract_host(""), None);
    }

    #[test]
    fn shift_decimal_matches_documented_examples() {
        assert_eq!(shift_decimal_by("43.5", -2).as_deref(), Some("0.435"));
        assert_eq!(shift_decimal_by("120", -2).as_deref(), Some("1.2"));
        assert_eq!(shift_decimal_by("1234", -2).as_deref(), Some("12.34"));
        assert_eq!(shift_decimal_by("25", -2).as_deref(), Some("0.25"));
        assert_eq!(shift_decimal_by("-1", -2).as_deref(), Some("-0.01"));
        assert_eq!(shift_decimal_by("0", -2).as_deref(), Some("0"));
        assert!(shift_decimal_by("abc", -2).is_none());
    }

    #[test]
    fn ratio_normalization_caps_over_limit() {
        assert_eq!(
            normalize_ratio("1.2"),
            (
                "1".to_owned(),
                Some("0".to_owned()),
                Some("1.2".to_owned()),
                Some(true)
            )
        );
        assert_eq!(
            normalize_ratio("0.25"),
            ("0.25".to_owned(), Some("0.75".to_owned()), None, None)
        );
        assert!(is_gt_one("1.01"));
        assert!(!is_gt_one("1.00"));
        assert!(!is_gt_one("0.5"));
    }

    #[test]
    fn json_decimal_and_int_helpers() {
        assert_eq!(
            json_decimal(&serde_json::json!("10.5")).as_deref(),
            Some("10.5")
        );
        assert_eq!(json_decimal(&serde_json::json!(42)).as_deref(), Some("42"));
        assert_eq!(json_decimal(&serde_json::json!("x")), None);
        assert!(is_decimal("10."));
        assert!(!is_decimal("10%"));
        assert_eq!(json_int(&serde_json::json!("42")), Some(42));
        assert_eq!(json_int(&serde_json::json!(7)), Some(7));
        assert_eq!(json_int(&serde_json::json!(1.5)), None);
    }

    #[test]
    fn decimal_field_tolerates_strings_and_numbers() {
        assert_eq!(
            decimal_field(&serde_json::json!("110.00")).as_deref(),
            Some("110.00")
        );
        assert_eq!(
            decimal_field(&serde_json::json!(100)).as_deref(),
            Some("100")
        );
        assert_eq!(decimal_field(&serde_json::json!("1e5")), None);
        let map: Map<String, Value> = serde_json::from_str(r#"{"a":"1.5","b":null}"#).unwrap();
        assert_eq!(opt_decimal(&map, "a"), Ok(Some("1.5".to_owned())));
        assert_eq!(opt_decimal(&map, "b"), Ok(None));
        assert_eq!(opt_decimal(&map, "missing"), Ok(None));
        assert_eq!(opt_string(&map, "a"), Ok(Some("1.5".to_owned())));
    }
}

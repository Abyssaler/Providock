//! B07 Codex 官方 App Server 只读账户能力适配器。
//!
//! 证据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B07（E1，官方
//! App Server 协议 `learn.chatgpt.com/docs/app-server`，源码固定提交
//! `openai/codex@8c0f7011674145da7e10627769fb221c74fbf7a0`，2026-10-11 抓取）。
//!
//! - 传输：JSON-RPC（`{"method","params","id"}` → `result`/`error`），默认 stdio，
//!   另有 `ws://`、`unix://`。
//! - **协议无数字版本号**，靠能力协商：先 `initialize`（`clientInfo` +
//!   `capabilities.experimentalApi=true` 才启用实验方法/字段）；响应返回
//!   `userAgent / codexHome / platformFamily / platformOs`。本适配器把协商结果写入
//!   evidence。
//! - 只读方法：`account/rateLimits/read`、`account/usage/read`（固定提交中两者均**未**
//!   被 `#[experimental]` 标注）。
//! - `RateLimitWindow = { usedPercent, windowDurationMins|null, resetsAt(unix 秒)|null }`：
//!   `usedPercent` 是 **0–100 的百分比**（源码多处按 `>= 100` 判满），本适配器换算为
//!   0–1 的比例；`duration_seconds = windowDurationMins * 60`。
//! - `account/usage/read` 的 `summary` 各值可为 null；`dailyUsageBuckets` 可为 null。
//!   **API-key-only / Bedrock 认证不支持 usage**：服务端回 JSON-RPC 错误
//!   “chatgpt authentication required …”，本适配器映射为 `unsupported`（能力缺失，
//!   不显示假数据），而不是暂时失败。
//!
//! 纯函数：不触网、不读文件、不接触凭据。App Server 自身持有认证，`RequestSpec`
//! 不含任何秘密（`auth = ServerHeld`）。

use crate::domain::Quality;
use crate::services::accounts::{FailureSignal, PeriodKind, QuotaUnit, QuotaWindow, SnapshotStatus};
use serde_json::Value;

pub const INITIALIZE_METHOD: &str = "initialize";
pub const RATE_LIMITS_METHOD: &str = "account/rateLimits/read";
pub const USAGE_METHOD: &str = "account/usage/read";
/// 证据标注（E1，无数字版本号——以 initialize 协商结果为准）。
pub const EVIDENCE: &str =
    "codex:appserver:E1:openai/codex@8c0f7011674145da7e10627769fb221c74fbf7a0";

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    WebSocket(String),
    Unix(String),
}

impl Default for Transport {
    fn default() -> Self {
        Self::Stdio
    }
}

/// App Server 自己持有认证；适配器不注入凭据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthScheme {
    ServerHeld,
}

/// JSON-RPC 请求：**不含秘密**。
#[derive(Debug, Clone, PartialEq)]
pub struct RequestSpec {
    pub transport: Transport,
    pub method: &'static str,
    pub request_id: i64,
    pub params: Value,
    pub auth: AuthScheme,
}

impl RequestSpec {
    /// 线上帧：`{"method","params","id"}`。
    pub fn to_wire(&self) -> Value {
        serde_json::json!({
            "method": self.method,
            "params": self.params,
            "id": self.request_id,
        })
    }
}

/// `initialize`：声明 `clientInfo` 与 `capabilities.experimentalApi`。
pub fn initialize(
    request_id: i64,
    client_name: &str,
    client_title: Option<&str>,
    client_version: &str,
    experimental_api: bool,
) -> RequestSpec {
    let mut client_info = serde_json::json!({
        "name": client_name,
        "version": client_version,
    });
    if let Some(title) = client_title {
        client_info["title"] = Value::String(title.to_owned());
    }
    RequestSpec {
        transport: Transport::Stdio,
        method: INITIALIZE_METHOD,
        request_id,
        params: serde_json::json!({
            "clientInfo": client_info,
            "capabilities": { "experimentalApi": experimental_api },
        }),
        auth: AuthScheme::ServerHeld,
    }
}

/// `account/rateLimits/read`。两参数省略时服务端按默认行为读取。
pub fn rate_limits_read(
    request_id: i64,
    supports_luna_reserve: bool,
    exclude_reset_credit_details: bool,
) -> RequestSpec {
    let mut params = serde_json::Map::new();
    if supports_luna_reserve {
        params.insert("supportsLunaReserve".to_owned(), Value::Bool(true));
    }
    if exclude_reset_credit_details {
        params.insert("excludeResetCreditDetails".to_owned(), Value::Bool(true));
    }
    RequestSpec {
        transport: Transport::Stdio,
        method: RATE_LIMITS_METHOD,
        request_id,
        params: Value::Object(params),
        auth: AuthScheme::ServerHeld,
    }
}

/// `account/usage/read`。`thread_id` 给定时读该 thread 的估算用量，否则读账户级。
pub fn usage_read(request_id: i64, thread_id: Option<&str>) -> RequestSpec {
    let params = match thread_id {
        Some(thread_id) => serde_json::json!({ "threadId": thread_id }),
        None => Value::Object(serde_json::Map::new()),
    };
    RequestSpec {
        transport: Transport::Stdio,
        method: USAGE_METHOD,
        request_id,
        params,
        auth: AuthScheme::ServerHeld,
    }
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodexError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("authentication required: {0}")]
    AuthRequired(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("incompatible: {0}")]
    Incompatible(String),
}

impl CodexError {
    pub fn status(&self) -> SnapshotStatus {
        match self {
            Self::Transport(_) => SnapshotStatus::Unavailable,
            Self::Protocol(_) | Self::Incompatible(_) => SnapshotStatus::Incompatible,
            Self::Rpc { .. } => SnapshotStatus::Unavailable,
            Self::AuthRequired(_) => SnapshotStatus::AuthRequired,
            Self::Unsupported(_) => SnapshotStatus::Unsupported,
        }
    }

    pub fn failure_signal(&self) -> FailureSignal {
        let mut signal = FailureSignal {
            status_hint: Some(self.status()),
            ..Default::default()
        };
        match self {
            Self::Transport(_) => signal.transport_failed = true,
            Self::Protocol(_) | Self::Incompatible(_) => signal.schema_changed = true,
            Self::Unsupported(_) => signal.unsupported = true,
            Self::Rpc { code, .. } => signal.business_code = Some(code.to_string()),
            Self::AuthRequired(_) => {}
        }
        signal
    }
}

// ---------------------------------------------------------------------------
// 响应：JSON-RPC 信封
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RpcOutcome {
    Ok(Value),
    Error(RpcError),
}

/// 解析一条 JSON-RPC 响应（`result` 或 `error`）。
pub fn parse_response(body: &[u8]) -> Result<RpcOutcome, CodexError> {
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| CodexError::Protocol(format!("response is not JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| CodexError::Protocol("JSON-RPC response is not an object".into()))?;
    if let Some(error) = obj.get("error") {
        let error = error
            .as_object()
            .ok_or_else(|| CodexError::Protocol("JSON-RPC error is not an object".into()))?;
        let code = error
            .get("code")
            .and_then(json_int)
            .ok_or_else(|| CodexError::Protocol("JSON-RPC error.code missing".into()))?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        return Ok(RpcOutcome::Error(RpcError { code, message }));
    }
    let result = obj
        .get("result")
        .ok_or_else(|| CodexError::Protocol("response has neither result nor error".into()))?;
    Ok(RpcOutcome::Ok(result.clone()))
}

fn result_of(outcome: RpcOutcome) -> Result<Value, CodexError> {
    match outcome {
        RpcOutcome::Ok(value) => Ok(value),
        RpcOutcome::Error(error) => Err(map_rpc_error(error)),
    }
}

/// JSON-RPC 错误 → 契约状态。
///
/// “chatgpt authentication required …” 是 API-key-only / Bedrock 的**能力缺失**
/// （`unsupported`），不是暂时失败；“codex account authentication required …” 是
/// 完全未登录（`auth_required`）。
fn map_rpc_error(error: RpcError) -> CodexError {
    let lowered = error.message.to_ascii_lowercase();
    if error.code == METHOD_NOT_FOUND {
        return CodexError::Unsupported(error.message);
    }
    if lowered.contains("chatgpt authentication required") {
        return CodexError::Unsupported(error.message);
    }
    if lowered.contains("codex account authentication required") {
        return CodexError::AuthRequired(error.message);
    }
    if lowered.contains("requires experimentalapi") {
        return CodexError::Unsupported(error.message);
    }
    if error.code == INVALID_PARAMS {
        return CodexError::Incompatible(error.message);
    }
    CodexError::Rpc {
        code: error.code,
        message: error.message,
    }
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InitializeInfo {
    pub user_agent: Option<String>,
    pub platform_family: Option<String>,
    pub platform_os: Option<String>,
    pub evidence: String,
}
pub fn parse_initialize(body: &[u8]) -> Result<InitializeInfo, CodexError> {
    let result = result_of(parse_response(body)?)?;
    let obj = result
        .as_object()
        .ok_or_else(|| CodexError::Incompatible("initialize result is not an object".into()))?;
    let user_agent = obj.get("userAgent").and_then(Value::as_str).map(str::to_owned);
    if user_agent.is_none() {
        return Err(CodexError::Incompatible(
            "initialize result.userAgent missing".into(),
        ));
    }
    let platform_family = obj
        .get("platformFamily")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let platform_os = obj
        .get("platformOs")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let evidence = format!(
        "{EVIDENCE}:user_agent={}",
        user_agent.as_deref().unwrap_or("unknown")
    );
    Ok(InitializeInfo {
        user_agent,
        platform_family,
        platform_os,
        evidence,
    })
}

// ---------------------------------------------------------------------------
// account/rateLimits/read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodexRateLimits {
    pub windows: Vec<QuotaWindow>,
    pub credits: Option<CreditsSnapshot>,
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    pub ordinary_usage_allowed: Option<bool>,
    pub evidence: String,
}

pub fn parse_rate_limits(body: &[u8]) -> Result<CodexRateLimits, CodexError> {
    let result = result_of(parse_response(body)?)?;
    let obj = result
        .as_object()
        .ok_or_else(|| CodexError::Incompatible("rate limits result is not an object".into()))?;
    let rate_limits = obj
        .get("rateLimits")
        .and_then(Value::as_object)
        .ok_or_else(|| CodexError::Incompatible("rateLimits missing".into()))?;

    let mut windows = Vec::new();
    // 多桶视图优先（键为 metered limit_id）；否则用向后兼容的单桶。
    match obj.get("rateLimitsByLimitId").and_then(Value::as_object) {
        Some(map) if !map.is_empty() => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(snapshot) = map.get(key).and_then(Value::as_object) {
                    let limit_id = snapshot
                        .get("limitId")
                        .and_then(Value::as_str)
                        .unwrap_or(key);
                    push_snapshot_windows(&mut windows, limit_id, snapshot)?;
                }
            }
        }
        _ => {
            let limit_id = rate_limits
                .get("limitId")
                .and_then(Value::as_str)
                .unwrap_or("codex");
            push_snapshot_windows(&mut windows, limit_id, rate_limits)?;
        }
    }

    Ok(CodexRateLimits {
        windows,
        credits: parse_credits(rate_limits.get("credits")),
        plan_type: rate_limits
            .get("planType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        account_id: obj.get("accountId").and_then(Value::as_str).map(str::to_owned),
        ordinary_usage_allowed: obj.get("ordinaryUsageAllowed").and_then(Value::as_bool),
        evidence: EVIDENCE.to_owned(),
    })
}

fn push_snapshot_windows(
    out: &mut Vec<QuotaWindow>,
    limit_id: &str,
    snapshot: &serde_json::Map<String, Value>,
) -> Result<(), CodexError> {
    let limit_name = snapshot.get("limitName").and_then(Value::as_str);
    for slot in ["primary", "secondary"] {
        let Some(window) = snapshot.get(slot).and_then(Value::as_object) else {
            continue;
        };
        let used_percent = window
            .get("usedPercent")
            .and_then(json_decimal)
            .ok_or_else(|| {
                CodexError::Incompatible(format!("{limit_id}.{slot}.usedPercent missing"))
            })?;
        let ratio = shift_decimal_by(&used_percent, -2).ok_or_else(|| {
            CodexError::Incompatible(format!("{limit_id}.{slot}.usedPercent not a decimal"))
        })?;
        let (used_ratio, remaining_ratio, raw_ratio, over_limit) = normalize_ratio(&ratio);
        let duration_seconds = window
            .get("windowDurationMins")
            .and_then(json_int)
            .and_then(|minutes| u64::try_from(minutes).ok())
            .and_then(|minutes| minutes.checked_mul(60));
        let resets_at = window.get("resetsAt").and_then(json_int).and_then(unix_to_rfc3339);

        let mut quota =
            QuotaWindow::new(format!("codex:account:{limit_id}:{slot}"), QuotaUnit::Percent);
        quota.label = Some(match limit_name {
            Some(name) => format!("{name} ({slot})"),
            None => format!("{limit_id} ({slot})"),
        });
        quota.resource = Some(limit_id.to_owned());
        quota.scope = Some("account".to_owned());
        quota.period_kind = Some(PeriodKind::Rolling);
        quota.duration_seconds = duration_seconds;
        quota.resets_at = resets_at;
        quota.used_ratio = Some(used_ratio);
        quota.remaining_ratio = remaining_ratio;
        quota.raw_ratio = raw_ratio;
        quota.over_limit = over_limit;
        quota.quality = Quality::Reported;
        quota.evidence_ref = Some(EVIDENCE.to_owned());
        out.push(quota);
    }
    Ok(())
}

fn parse_credits(value: Option<&Value>) -> Option<CreditsSnapshot> {
    let obj = value.and_then(Value::as_object)?;
    Some(CreditsSnapshot {
        has_credits: obj.get("hasCredits").and_then(Value::as_bool).unwrap_or(false),
        unlimited: obj.get("unlimited").and_then(Value::as_bool).unwrap_or(false),
        balance: obj.get("balance").and_then(Value::as_str).map(str::to_owned),
    })
}

// ---------------------------------------------------------------------------
// account/usage/read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    Tokens,
    Seconds,
    Days,
    CreditsMicros,
    UsdMicros,
}

impl MetricUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Seconds => "seconds",
            Self::Days => "days",
            Self::CreditsMicros => "credits_micros",
            Self::UsdMicros => "usd_micros",
        }
    }
}

/// 账户级 usage 指标（DATA_CONTRACTS §7 `usage_metrics` 语义，十进制字符串）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UsageMetric {
    pub name: &'static str,
    pub value: String,
    pub unit: MetricUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DailyBucket {
    pub start_date: String,
    pub tokens: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ThreadUsageGroup {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub speed: Option<String>,
    pub estimated_usage_credits_micros: Option<String>,
    pub net_new_input_tokens: Option<String>,
    pub cached_input_tokens: Option<String>,
    pub input_tokens: Option<String>,
    pub output_tokens: Option<String>,
    pub total_tokens: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ThreadUsage {
    pub thread_id: String,
    pub estimated_usage_credits_micros: Option<String>,
    pub estimated_usage_usd_micros: Option<String>,
    pub groups: Vec<ThreadUsageGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodexUsage {
    pub metrics: Vec<UsageMetric>,
    pub daily_buckets: Vec<DailyBucket>,
    pub thread_usage: Option<ThreadUsage>,
    pub evidence: String,
}

pub fn parse_usage(body: &[u8]) -> Result<CodexUsage, CodexError> {
    let result = result_of(parse_response(body)?)?;
    let obj = result
        .as_object()
        .ok_or_else(|| CodexError::Incompatible("usage result is not an object".into()))?;
    let summary = obj
        .get("summary")
        .and_then(Value::as_object)
        .ok_or_else(|| CodexError::Incompatible("usage summary missing".into()))?;

    let mut metrics = Vec::new();
    for (wire, name, unit) in [
        ("lifetimeTokens", "lifetime_tokens", MetricUnit::Tokens),
        ("peakDailyTokens", "peak_daily_tokens", MetricUnit::Tokens),
        (
            "longestRunningTurnSec",
            "longest_running_turn_sec",
            MetricUnit::Seconds,
        ),
        ("currentStreakDays", "current_streak_days", MetricUnit::Days),
        ("longestStreakDays", "longest_streak_days", MetricUnit::Days),
    ] {
        if let Some(value) = summary.get(wire).and_then(json_decimal) {
            metrics.push(UsageMetric { name, value, unit });
        }
    }

    let daily_buckets = obj
        .get("dailyUsageBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let item = item.as_object()?;
                    let start_date = item.get("startDate")?.as_str()?.to_owned();
                    let tokens = json_decimal(item.get("tokens")?)?;
                    Some(DailyBucket { start_date, tokens })
                })
                .collect()
        })
        .unwrap_or_default();

    let thread_usage = match obj.get("threadUsage") {
        Some(Value::Object(map)) => Some(parse_thread_usage(map)?),
        _ => None,
    };

    Ok(CodexUsage {
        metrics,
        daily_buckets,
        thread_usage,
        evidence: EVIDENCE.to_owned(),
    })
}

fn parse_thread_usage(map: &serde_json::Map<String, Value>) -> Result<ThreadUsage, CodexError> {
    let thread_id = map
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| CodexError::Incompatible("threadUsage.threadId missing".into()))?
        .to_owned();
    let groups = map
        .get("groups")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object())
                .map(|group| ThreadUsageGroup {
                    model: group.get("model").and_then(Value::as_str).map(str::to_owned),
                    reasoning_effort: group
                        .get("reasoningEffort")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    speed: group.get("speed").and_then(Value::as_str).map(str::to_owned),
                    estimated_usage_credits_micros: group
                        .get("estimatedUsageCreditsMicros")
                        .and_then(json_decimal),
                    net_new_input_tokens: group.get("netNewInputTokens").and_then(json_decimal),
                    cached_input_tokens: group.get("cachedInputTokens").and_then(json_decimal),
                    input_tokens: group.get("inputTokens").and_then(json_decimal),
                    output_tokens: group.get("outputTokens").and_then(json_decimal),
                    total_tokens: group.get("totalTokens").and_then(json_decimal),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ThreadUsage {
        thread_id,
        estimated_usage_credits_micros: map
            .get("estimatedUsageCreditsMicros")
            .and_then(json_decimal),
        estimated_usage_usd_micros: map.get("estimatedUsageUsdMicros").and_then(json_decimal),
        groups,
    })
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// Unix 秒 → RFC3339（`Z`）。越界返回 None，不猜时间。
fn unix_to_rfc3339(seconds: i64) -> Option<String> {
    let dt = time::OffsetDateTime::from_unix_timestamp(seconds).ok()?;
    dt.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// 十进制字符串乘以 10^places（`places>0`）或除以 10^places（`places<0`），精确。
///
/// `"25"` 乘 -2 → `"0.25"`；`"1234"` 乘 -2 → `"12.34"`。
fn shift_decimal_by(value: &str, places: i32) -> Option<String> {
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

fn normalize_ratio(ratio: &str) -> (String, Option<String>, Option<String>, Option<bool>) {
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

fn one_minus(ratio: &str) -> Option<String> {
    use crate::domain::decimal::decimal_add;
    let negated = match ratio.strip_prefix('-') {
        Some(rest) => rest.to_owned(),
        None => format!("-{ratio}"),
    };
    decimal_add("1", &negated)
}

fn is_gt_one(value: &str) -> bool {
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

fn json_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn json_decimal(value: &Value) -> Option<String> {
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

fn is_decimal(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let mut parts = body.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    if int.is_empty() && frac.is_empty() {
        return false;
    }
    int.chars().all(|c| c.is_ascii_digit()) && frac.chars().all(|c| c.is_ascii_digit())
}

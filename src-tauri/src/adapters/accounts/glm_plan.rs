//! B05 GLM Coding Plan 只读适配器（智谱 bigmodel / 国际 Z.AI）。
//!
//! 证据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B05（E2）；
//! 官方插件 `zai-coding-plugins@0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254`
//! `plugins/glm-plan-usage/.../query-usage.mjs`（2026-10-11 抓取）：
//!
//! - host 由 `ANTHROPIC_BASE_URL` 的 origin 决定：`api.z.ai` = 国际；
//!   `open.bigmodel.cn` / `dev.bigmodel.cn` = 中国；其它 host 报错。**精确匹配域名**，
//!   不用子串判定。
//! - 路径：`/api/monitor/usage/quota/limit`（无查询串）、
//!   `/api/monitor/usage/model-usage`、`/api/monitor/usage/tool-usage`
//!   （后两者追加 `?startTime=<yyyy-MM-dd HH:mm:ss>&endTime=<...>`，官方插件用
//!   `encodeURIComponent` 编码；插件取本地时区，本适配器按 UTC 格式化，时区偏移由
//!   服务层在调用前决定）。
//! - `Authorization` 为**原 token，不加 `Bearer`**（源码头是 `'Authorization': authToken`）；
//!   本适配器只声明 scheme，凭据注入在服务层。
//! - 响应取 `json.data || json`；`limits[]` 按 `type` 归一：`TOKENS_LIMIT` → 5 小时
//!   Token 窗口百分比；`TIME_LIMIT` → 月度 MCP 用量。
//!
//! 语义边界（契约 §8）：只拿到 `percentage`（0–100）时只能填 `used_ratio` /
//! `remaining_ratio`，**绝不反推绝对 Token/请求数**。超过 100% 记 `raw_ratio` +
//! `over_limit`，`used_ratio` 封顶为 1。
//!
//! 纯函数：不触网、不读文件、不接触凭据；`RequestSpec` 不装载任何秘密。
//! 请求规格与十进制/比例工具统一取自 [`crate::adapters::accounts::common`]。

use crate::adapters::accounts::common::{self, AuthPlacement, RequestSpec};
use crate::domain::Quality;
use crate::services::accounts::{FailureSignal, QuotaUnit, QuotaWindow, SnapshotStatus};
use serde_json::Value;

/// 证据标注（E2：官方客户端/插件源码中的只读接口，兼容可能变化）。
pub const EVIDENCE: &str =
    "glm:coding_plan:E2:zai-coding-plugins@0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254";

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

/// 平台区域由 host 精确决定（不是由调用方随意指定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlmRegion {
    China,
    Global,
}

impl GlmRegion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::China => "cn",
            Self::Global => "global",
        }
    }

    /// 该区域的规范主 host（`dev.bigmodel.cn` 也属中国区，但请求用配置的实际 host）。
    pub fn host(self) -> &'static str {
        match self {
            Self::China => "open.bigmodel.cn",
            Self::Global => "api.z.ai",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlmEndpoint {
    QuotaLimit,
    ModelUsage,
    ToolUsage,
}

impl GlmEndpoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuotaLimit => "quota_limit",
            Self::ModelUsage => "model_usage",
            Self::ToolUsage => "tool_usage",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::QuotaLimit => "/api/monitor/usage/quota/limit",
            Self::ModelUsage => "/api/monitor/usage/model-usage",
            Self::ToolUsage => "/api/monitor/usage/tool-usage",
        }
    }
}

/// `model-usage` / `tool-usage` 的查询时间窗（epoch 毫秒，闭区间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start_ms: i64,
    pub end_ms: i64,
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// 适配错误 → 契约状态与失败信号（ACCOUNT_ADAPTERS §5）。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GlmError {
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(String),
    #[error("unrecognized GLM host: {0}")]
    UnrecognizedHost(String),
    #[error("HTTP {status}")]
    Http { status: u16 },
    #[error("business error {code}: {message}")]
    Business { code: String, message: String },
    #[error("schema changed: {0}")]
    SchemaChanged(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl GlmError {
    pub fn status(&self) -> SnapshotStatus {
        match self {
            Self::InvalidBaseUrl(_) => SnapshotStatus::Incompatible,
            Self::UnrecognizedHost(_) => SnapshotStatus::Unsupported,
            Self::Http { status } => match status {
                401 | 403 => SnapshotStatus::AuthRequired,
                429 => SnapshotStatus::RateLimited,
                _ => SnapshotStatus::Unavailable,
            },
            Self::Business { code, .. } => match code.as_str() {
                "401" | "403" | "40101" => SnapshotStatus::AuthRequired,
                "429" | "42901" => SnapshotStatus::RateLimited,
                _ => SnapshotStatus::Unavailable,
            },
            Self::SchemaChanged(_) => SnapshotStatus::Incompatible,
            Self::Unsupported(_) => SnapshotStatus::Unsupported,
        }
    }

    /// 直接喂给 `services::accounts::failure_record` 的信号。
    pub fn failure_signal(&self) -> FailureSignal {
        let mut signal = FailureSignal {
            status_hint: Some(self.status()),
            ..Default::default()
        };
        match self {
            Self::Http { status } => signal.http_status = Some(*status),
            Self::Business { code, .. } => signal.business_code = Some(code.clone()),
            Self::SchemaChanged(_) => signal.schema_changed = true,
            Self::Unsupported(_) => signal.unsupported = true,
            Self::InvalidBaseUrl(_) | Self::UnrecognizedHost(_) => {}
        }
        signal
    }
}

// ---------------------------------------------------------------------------
// 请求构造
// ---------------------------------------------------------------------------

struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

/// 只解析 `scheme://host[:port]` 段；拒绝 userinfo 与非法 scheme/port。
fn parse_origin(raw: &str) -> Result<Origin, GlmError> {
    let raw = raw.trim();
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| GlmError::InvalidBaseUrl(raw.to_owned()))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(GlmError::InvalidBaseUrl(raw.to_owned()));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(GlmError::InvalidBaseUrl(raw.to_owned()));
    }
    let (host, port) = common::split_host_port(authority)
        .ok_or_else(|| GlmError::InvalidBaseUrl(raw.to_owned()))?;
    Ok(Origin {
        scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// 由 `ANTHROPIC_BASE_URL` 判定区域（host 精确匹配）。
pub fn region_for_base(base_url: &str) -> Result<GlmRegion, GlmError> {
    let origin = parse_origin(base_url)?;
    match origin.host.as_str() {
        "api.z.ai" => Ok(GlmRegion::Global),
        "open.bigmodel.cn" | "dev.bigmodel.cn" => Ok(GlmRegion::China),
        other => Err(GlmError::UnrecognizedHost(other.to_owned())),
    }
}

/// 构造请求。
///
/// - `quota/limit` 不接受时间窗（插件不带查询串）；传了时间窗是调用方错误。
/// - `model-usage` / `tool-usage` 必须给时间窗。
/// - URL 用配置 base 的实际 origin 拼路径（保持用户配置的 host，不做跨 host 回落）。
pub fn build_request(
    base_url: &str,
    endpoint: GlmEndpoint,
    window: Option<TimeWindow>,
) -> Result<RequestSpec, GlmError> {
    let origin = parse_origin(base_url)?;
    match origin.host.as_str() {
        "api.z.ai" | "open.bigmodel.cn" | "dev.bigmodel.cn" => {}
        other => return Err(GlmError::UnrecognizedHost(other.to_owned())),
    }
    let mut authority = origin.host.clone();
    if let Some(port) = origin.port {
        authority.push_str(&format!(":{port}"));
    }
    let mut path = endpoint.path().to_owned();
    match endpoint {
        GlmEndpoint::QuotaLimit => {
            if window.is_some() {
                return Err(GlmError::InvalidBaseUrl(
                    "quota/limit takes no query window".into(),
                ));
            }
        }
        GlmEndpoint::ModelUsage | GlmEndpoint::ToolUsage => {
            let window = window.ok_or_else(|| {
                GlmError::InvalidBaseUrl("model/tool usage requires a time window".into())
            })?;
            let start = format_query_time(window.start_ms).ok_or_else(|| {
                GlmError::InvalidBaseUrl(format!("time out of range: {}", window.start_ms))
            })?;
            let end = format_query_time(window.end_ms).ok_or_else(|| {
                GlmError::InvalidBaseUrl(format!("time out of range: {}", window.end_ms))
            })?;
            path.push_str(&format!(
                "?startTime={}&endTime={}",
                percent_encode(&start),
                percent_encode(&end)
            ));
        }
    }
    let mut spec = RequestSpec::https(authority, path).with_headers(vec![
        ("Accept-Language".to_owned(), "en-US,en".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ]);
    // GLM 用配置 base 的 scheme（规范 host 都是 https；仅当用户显式配 http 才跟随）。
    spec.scheme = origin.scheme;
    // 原 token，不加 Bearer（服务层按此注入）。
    spec.auth = Some(AuthPlacement::raw_token());
    Ok(spec)
}

/// epoch 毫秒 → 插件使用的 `yyyy-MM-dd HH:mm:ss`（本适配器按 UTC；服务层决定本地偏移）。
fn format_query_time(ms: i64) -> Option<String> {
    let secs = ms.div_euclid(1000);
    let dt = time::OffsetDateTime::from_unix_timestamp(secs).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    ))
}

/// `encodeURIComponent` 语义：保留 `A-Za-z0-9-_.!~*'()`，其余百分号编码。
fn percent_encode(input: &str) -> String {
    const KEEP: &[u8] = b"-_.!~*'()";
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

/// 解析结果。`model-usage` / `tool-usage` 暂无 schema 依据，只回诊断不回填数据。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GlmReport {
    pub windows: Vec<QuotaWindow>,
    pub plan_level: Option<String>,
    pub diagnostics: Vec<String>,
    pub evidence: String,
}

/// `status` / `body` → [`GlmReport`]；错误按契约状态映射。
pub fn parse_response(
    endpoint: GlmEndpoint,
    status: u16,
    body: &[u8],
) -> Result<GlmReport, GlmError> {
    if status >= 400 {
        return Err(GlmError::Http { status });
    }
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| GlmError::SchemaChanged(format!("body is not JSON: {e}")))?;
    check_business(&root)?;
    match endpoint {
        GlmEndpoint::QuotaLimit => parse_quota_limit(&root),
        GlmEndpoint::ModelUsage | GlmEndpoint::ToolUsage => Ok(GlmReport {
            windows: Vec::new(),
            plan_level: None,
            diagnostics: vec![format!(
                "{}:detail_stats_parse_not_in_scope",
                endpoint.as_str()
            )],
            evidence: EVIDENCE.to_owned(),
        }),
    }
}

/// HTTP200 业务错误：`success=false` 或 `code` 不是 0/200。
///
/// GLM 的 `code` 语义未在证据中固定；按“非成功码即失败”处理，避免把拒绝当成功。
fn check_business(root: &Value) -> Result<(), GlmError> {
    let Some(obj) = root.as_object() else {
        return Ok(());
    };
    if obj.get("success") == Some(&Value::Bool(false)) {
        let message = obj
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("success=false");
        return Err(GlmError::Business {
            code: "success_false".to_owned(),
            message: message.to_owned(),
        });
    }
    if let Some(code) = obj.get("code").and_then(common::json_decimal) {
        if code != "0" && code != "200" {
            let message = obj
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            return Err(GlmError::Business { code, message });
        }
    }
    Ok(())
}

fn parse_quota_limit(root: &Value) -> Result<GlmReport, GlmError> {
    let data = root.get("data").unwrap_or(root);
    let data = data
        .as_object()
        .ok_or_else(|| GlmError::SchemaChanged("quota data is not an object".into()))?;
    let plan_level = data.get("level").and_then(Value::as_str).map(str::to_owned);
    let limits = data
        .get("limits")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            GlmError::SchemaChanged("quota data.limits missing or not an array".into())
        })?;

    let mut windows = Vec::new();
    let mut diagnostics = Vec::new();
    let mut occurrences: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    for item in limits {
        let Some(obj) = item.as_object() else {
            diagnostics.push("limit_item_not_object".to_owned());
            continue;
        };
        let raw_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
        let (key, label, resource) = match raw_type.to_ascii_uppercase().as_str() {
            "TOKENS_LIMIT" => ("5h", "Token usage (5h)", "tokens"),
            "TIME_LIMIT" => ("month", "MCP usage (1 month)", "mcp"),
            other => {
                diagnostics.push(format!("unknown_limit_type:{other}"));
                continue;
            }
        };
        let percentage = obj
            .get("percentage")
            .and_then(common::json_decimal)
            .ok_or_else(|| GlmError::SchemaChanged(format!("{raw_type}.percentage missing")))?;
        let ratio = common::shift_decimal_by(&percentage, -2).ok_or_else(|| {
            GlmError::SchemaChanged(format!("{raw_type}.percentage not a decimal: {percentage}"))
        })?;

        let (used_ratio, remaining_ratio, raw_ratio, over_limit) = common::normalize_ratio(&ratio);
        let count = occurrences.entry(key).or_insert(0);
        *count += 1;
        let mut window_id = format!("glm:coding_plan:account:{key}:{resource}");
        if *count > 1 {
            window_id.push_str(&format!(":{}", *count));
        }
        let mut window = QuotaWindow::new(window_id, QuotaUnit::Percent);
        window.label = Some(label.to_owned());
        window.resource = Some(resource.to_owned());
        window.scope = Some("account".to_owned());
        window.used_ratio = Some(used_ratio);
        window.remaining_ratio = remaining_ratio;
        window.raw_ratio = raw_ratio;
        window.over_limit = over_limit;
        window.quality = Quality::Reported;
        window.evidence_ref = Some(EVIDENCE.to_owned());
        windows.push(window);
    }

    Ok(GlmReport {
        windows,
        plan_level,
        diagnostics,
        evidence: EVIDENCE.to_owned(),
    })
}

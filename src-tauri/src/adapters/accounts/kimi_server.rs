//! B06 Kimi 本机 Server 只读适配器。
//!
//! 证据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B06 与
//! `docs/research/evidence/a08-kimi-official.md` §7（E1，官方文档 `cli-server-api`
//! 与其权威副本 `docs/zh/reference/server-api.md`；**官方标注实验接口**，端点与字段
//! 可能随版本变化，须按实例 `/openapi.json` 协商）。
//!
//! - 默认地址 `http://127.0.0.1:58627`；端口占用时顺延，`--port/--host` 可改。
//!   本适配器只接受 **loopback** base，绝不把本机令牌发往非本机 host。
//! - 鉴权 `Authorization: Bearer <本机令牌>`；`GET /api/v1/healthz` 与 `OPTIONS` 免鉴权。
//! - 统一信封 `{ code, msg, data, request_id }`；**HTTP 状态几乎总是 200，业务结果看
//!   `code`**（`code != 0` 即业务错误）；认证失败 HTTP 401 + `code=40101`，
//!   限流 HTTP 429 + `code=42901`。
//! - `GET /api/v1/meta` 协商版本与 capabilities。
//! - `GET /api/v1/oauth/usage` → `data.{kind:"ok", quota}` 或 `{kind:"error", ...}`
//!   （上游失败以 in-band 返回，**不使信封失败**）。`quota.usages` 的
//!   `limit5h / limit7d / monthTotal / monthCode`（存在即显示）各 `{usedRatio, resetAt?}`
//!   是 4 个独立窗口；`extraUsage`（可为 null）是按量付费钱包，金额单位为 **cents**。
//!
//! 纯函数：不触网、不读文件、不接触凭据；`RequestSpec` 不装载任何秘密。
//! 请求规格与十进制/比例工具统一取自 [`crate::adapters::accounts::common`]。

use crate::adapters::accounts::common::{self, AuthPlacement, RequestSpec};
use crate::domain::Quality;
use crate::services::accounts::{
    BalanceEntry, BalanceKind, FailureSignal, PeriodKind, QuotaUnit, QuotaWindow, SnapshotStatus,
};
use serde_json::Value;

/// 默认 base（用户可换成配置的 loopback 实例/端口）。
pub const DEFAULT_BASE: &str = "http://127.0.0.1:58627";
/// 证据标注（E1，官方标注实验接口）。
pub const EVIDENCE: &str = "kimi:local_server:E1:experimental";

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KimiEndpoint {
    Meta,
    OauthUsage,
}

impl KimiEndpoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::OauthUsage => "oauth_usage",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Meta => "/api/v1/meta",
            Self::OauthUsage => "/api/v1/oauth/usage",
        }
    }
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KimiError {
    #[error("invalid base url: {0}")]
    InvalidBase(String),
    #[error("refusing to send credentials to non-loopback host: {0}")]
    NonLoopbackHost(String),
    #[error("HTTP {status}")]
    Http { status: u16 },
    #[error("business error {code}: {message}")]
    Business { code: String, message: String },
    #[error("upstream usage error: {message}")]
    UpstreamError {
        message: String,
        status: Option<u16>,
    },
    #[error("schema changed: {0}")]
    SchemaChanged(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl KimiError {
    pub fn status(&self) -> SnapshotStatus {
        match self {
            Self::InvalidBase(_) => SnapshotStatus::Incompatible,
            Self::NonLoopbackHost(_) => SnapshotStatus::Unsupported,
            Self::Http { status } => match status {
                401 | 403 => SnapshotStatus::AuthRequired,
                429 => SnapshotStatus::RateLimited,
                _ => SnapshotStatus::Unavailable,
            },
            Self::Business { code, .. } => match code.as_str() {
                "40101" | "401" | "403" => SnapshotStatus::AuthRequired,
                "42901" | "429" => SnapshotStatus::RateLimited,
                _ => SnapshotStatus::Unavailable,
            },
            // 上游失败以 in-band 返回：暂时不可用，不是无能力。
            Self::UpstreamError { .. } => SnapshotStatus::Unavailable,
            Self::SchemaChanged(_) => SnapshotStatus::Incompatible,
            Self::Unsupported(_) => SnapshotStatus::Unsupported,
        }
    }

    pub fn failure_signal(&self) -> FailureSignal {
        let mut signal = FailureSignal {
            status_hint: Some(self.status()),
            ..Default::default()
        };
        match self {
            Self::Http { status } => signal.http_status = Some(*status),
            Self::Business { code, .. } => signal.business_code = Some(code.clone()),
            Self::SchemaChanged(_) => signal.schema_changed = true,
            Self::Unsupported(_) | Self::NonLoopbackHost(_) => signal.unsupported = true,
            Self::InvalidBase(_) | Self::UpstreamError { .. } => {}
        }
        signal
    }
}

// ---------------------------------------------------------------------------
// 请求构造
// ---------------------------------------------------------------------------

/// 只接受显式 loopback base（`127.0.0.1` / `::1` / `localhost` / `127.x`，http）。
pub fn build_request(base_url: &str, endpoint: KimiEndpoint) -> Result<RequestSpec, KimiError> {
    let authority = normalize_loopback_authority(base_url)?;
    let mut spec = RequestSpec::https(authority, endpoint.path())
        .with_headers(vec![("Accept".to_owned(), "application/json".to_owned())]);
    // 本机 Server 固定 http（loopback）；服务层的「仅 HTTPS 外网 + loopback http 例外」据此放行。
    spec.scheme = "http".into();
    spec.auth = Some(AuthPlacement::bearer());
    Ok(spec)
}

/// `scheme://host[:port]` → 规范化 `host[:port]`（IPv6 字面量带方括号）；拒绝非 loopback / 非 http。
fn normalize_loopback_authority(raw: &str) -> Result<String, KimiError> {
    let raw = raw.trim().trim_end_matches('/');
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| KimiError::InvalidBase(raw.to_owned()))?;
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(KimiError::InvalidBase(format!(
            "loopback server base must use http: {raw}"
        )));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(KimiError::InvalidBase(raw.to_owned()));
    }
    let (host, port) =
        common::split_host_port(authority).ok_or_else(|| KimiError::InvalidBase(raw.to_owned()))?;
    if !is_loopback(&host) {
        return Err(KimiError::NonLoopbackHost(host));
    }
    // IPv6 字面量在 URL 中必须带方括号。
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    match port {
        Some(port) => Ok(format!("{host}:{port}")),
        None => Ok(host),
    }
}

fn is_loopback(host: &str) -> bool {
    let host = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    host == "localhost" || host == "::1" || host == "127.0.0.1" || host.starts_with("127.")
}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

/// 统一信封。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub code: i64,
    pub msg: Option<String>,
    pub data: Option<Value>,
    pub request_id: Option<String>,
}

fn parse_envelope(status: u16, body: &[u8]) -> Result<Envelope, KimiError> {
    if status >= 400 {
        return Err(KimiError::Http { status });
    }
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| KimiError::SchemaChanged(format!("body is not JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| KimiError::SchemaChanged("envelope is not an object".into()))?;
    let code = obj.get("code").and_then(common::json_int).ok_or_else(|| {
        KimiError::SchemaChanged("envelope.code missing or not an integer".into())
    })?;
    let msg = obj.get("msg").and_then(Value::as_str).map(str::to_owned);
    let request_id = obj
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let data = obj.get("data").cloned();
    if code != 0 {
        return Err(KimiError::Business {
            code: code.to_string(),
            message: msg.unwrap_or_default(),
        });
    }
    Ok(Envelope {
        code,
        msg,
        data,
        request_id,
    })
}

/// `/api/v1/meta` 的协商结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KimiMeta {
    pub server_version: Option<String>,
    pub server_id: Option<String>,
    pub backend: Option<String>,
    /// `capabilities` 中值为 `true` 的键。
    pub capabilities: Vec<String>,
    pub features: Vec<String>,
    pub evidence: String,
}

pub fn parse_meta(status: u16, body: &[u8]) -> Result<KimiMeta, KimiError> {
    let envelope = parse_envelope(status, body)?;
    let data = envelope
        .data
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| KimiError::SchemaChanged("meta data missing".into()))?;
    let server_version = data
        .get("server_version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let server_id = data
        .get("server_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let backend = data
        .get("backend")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let capabilities = data
        .get("capabilities")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter(|(_, value)| value.as_bool() == Some(true))
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default();
    let features = data
        .get("features")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let evidence = format!(
        "{EVIDENCE}:server_version={}",
        server_version.as_deref().unwrap_or("unknown")
    );
    Ok(KimiMeta {
        server_version,
        server_id,
        backend,
        capabilities,
        features,
        evidence,
    })
}

/// `/api/v1/oauth/usage` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KimiUsageReport {
    pub windows: Vec<QuotaWindow>,
    pub balances: Vec<BalanceEntry>,
    pub diagnostics: Vec<String>,
    pub evidence: String,
}

/// `usages` 的窗口顺序固定：5h、7d、月总额、月代码。
const WINDOWS: [(&str, &str, &str, PeriodKind, Option<u64>); 4] = [
    (
        "limit5h",
        "5h",
        "5-hour rolling",
        PeriodKind::Rolling,
        Some(18_000),
    ),
    (
        "limit7d",
        "7d",
        "7-day rolling",
        PeriodKind::Rolling,
        Some(604_800),
    ),
    (
        "monthTotal",
        "month_total",
        "Monthly total (calendar)",
        PeriodKind::Calendar,
        None,
    ),
    (
        "monthCode",
        "month_code",
        "Monthly code (calendar)",
        PeriodKind::Calendar,
        None,
    ),
];

pub fn parse_oauth_usage(status: u16, body: &[u8]) -> Result<KimiUsageReport, KimiError> {
    let envelope = parse_envelope(status, body)?;
    let data = envelope
        .data
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| KimiError::SchemaChanged("oauth usage data missing".into()))?;
    let kind = data
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| KimiError::SchemaChanged("oauth usage kind missing".into()))?;
    match kind {
        "ok" => {}
        "error" => {
            let message = data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream usage error")
                .to_owned();
            let upstream_status = data
                .get("status")
                .and_then(common::json_int)
                .and_then(|value| u16::try_from(value).ok());
            return Err(KimiError::UpstreamError {
                message,
                status: upstream_status,
            });
        }
        other => {
            return Err(KimiError::SchemaChanged(format!(
                "unknown oauth usage kind: {other}"
            )))
        }
    }
    let quota = data
        .get("quota")
        .and_then(Value::as_object)
        .ok_or_else(|| KimiError::SchemaChanged("quota missing".into()))?;
    let usages = quota
        .get("usages")
        .and_then(Value::as_object)
        .ok_or_else(|| KimiError::SchemaChanged("quota.usages missing".into()))?;

    let mut diagnostics = Vec::new();
    let mut windows = Vec::new();
    for (field, key, label, period_kind, duration) in WINDOWS {
        let Some(item) = usages.get(field) else {
            continue;
        };
        let Some(obj) = item.as_object() else {
            diagnostics.push(format!("{field}_not_object"));
            continue;
        };
        let used_ratio = obj
            .get("usedRatio")
            .and_then(common::json_decimal)
            .ok_or_else(|| KimiError::SchemaChanged(format!("{field}.usedRatio missing")))?;
        let (used, remaining, raw, over) = common::normalize_ratio(&used_ratio);
        let mut window = QuotaWindow::new(
            format!("kimi:coding_plan:account:{key}"),
            QuotaUnit::Percent,
        );
        window.label = Some(label.to_owned());
        window.resource = Some(field.to_owned());
        window.scope = Some("account".to_owned());
        window.period_kind = Some(period_kind);
        window.duration_seconds = duration;
        window.resets_at = obj
            .get("resetAt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        window.used_ratio = Some(used);
        window.remaining_ratio = remaining;
        window.raw_ratio = raw;
        window.over_limit = over;
        window.quality = Quality::Reported;
        window.evidence_ref = Some(EVIDENCE.to_owned());
        windows.push(window);
    }

    let mut balances = Vec::new();
    if let Some(extra) = quota.get("extraUsage").and_then(Value::as_object) {
        let currency = extra
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("USD")
            .to_owned();
        if let Some(cents) = extra.get("balanceCents").and_then(common::json_int) {
            let amount = cents_to_decimal(cents)
                .ok_or_else(|| KimiError::SchemaChanged("balanceCents out of range".into()))?;
            let mut balance = BalanceEntry::new(BalanceKind::Available, amount, currency.clone())
                .map_err(|e| KimiError::SchemaChanged(e.to_string()))?
                .with_formula("available = balanceCents / 100");
            balance.wallet_scope = Some("extra_usage".to_owned());
            balances.push(balance);
        }
        if let Some(cents) = extra.get("totalCents").and_then(common::json_int) {
            let amount = cents_to_decimal(cents)
                .ok_or_else(|| KimiError::SchemaChanged("totalCents out of range".into()))?;
            let mut balance = BalanceEntry::new(BalanceKind::Credit, amount, currency)
                .map_err(|e| KimiError::SchemaChanged(e.to_string()))?
                .with_formula("total = totalCents / 100");
            balance.wallet_scope = Some("extra_usage".to_owned());
            balances.push(balance);
        }
    }

    Ok(KimiUsageReport {
        windows,
        balances,
        diagnostics,
        evidence: format!("{EVIDENCE}:oauth/usage"),
    })
}

/// cents → 元，精确除 100（`1234 → "12.34"`）。
fn cents_to_decimal(cents: i64) -> Option<String> {
    common::shift_decimal_by(&cents.to_string(), -2)
}

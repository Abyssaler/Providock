//! Kimi Code Coding Key 直连通道（审计编号 B-04，`/coding/v1/usages`）。
//!
//! 证据：
//! - E2：官方 CLI 源码 `MoonshotAI/kimi-code@ c9f01f07388abc2cdca40c9c5267f58c65086710`
//!   `packages/oauth/src/managed-usage.ts`——`DEFAULT_KIMI_CODE_BASE_URL =
//!   'https://api.kimi.com/coding/v1'`、`GLOBAL_KIMI_CODE_BASE_URL =
//!   'https://api.kimi.ai/coding/v1'`，请求 `${base}/usages`，头
//!   `Authorization: Bearer <accessToken>` + `Accept: application/json`；原始 payload
//!   为 snake_case：`usages.{limit_5h,limit_7d,limit_month_total,limit_month_code}` 各
//!   `{used_ratio(0–1, 可缺省窗口), reset_time?}`。
//! - 2026-10-12 已用真实凭据验证（仅限 `GET https://api.kimi.com/coding/v1/usages` 的
//!   成功与 401 形态）：成功响应另含 `usage` / `limits`（camelCase 明细）与
//!   `booster_wallet`；无效凭据返回 HTTP 401
//!   `{"error":{"message":...,"type":"invalid_authentication_error"}}`。
//!   验证账户无 `limit_month_total` / `limit_month_code`，解析必须容忍月窗缺失。
//!
//! 语义边界：
//! - 窗口映射与 B06 本机 Server 通道一致（window_id / label / period_kind /
//!   duration_seconds 同源），两通道互为上游，快照窗口身份连续。
//! - `used_ratio` 缺失或不可解析 ⇒ schema 变化（不补零）；`reset_time` 仅接受 RFC3339
//!   （容忍有无小数秒），非法值记诊断并保持未知。
//! - `booster_wallet` 本轮**不解析为余额**：实测形态（`status: STATUS_DISABLED` 等）与
//!   CLI 读取的 `boosterWallet` schema 不一致，语义证据不足；容忍其存在。
//! - 顶层 `usage` / `limits` 明细字段本轮不解析（CLI 未读取，语义未登记）。
//!
//! 纯函数：不触网、不读文件、不接触凭据；`RequestSpec` 不装载任何秘密。
//! 请求规格与十进制/比例工具统一取自 [`crate::adapters::accounts::common`]。

use crate::adapters::accounts::common::{self, AuthPlacement, RequestSpec};
use crate::domain::Quality;
use crate::services::accounts::{
    FailureSignal, PeriodKind, QuotaUnit, QuotaWindow, SnapshotStatus,
};
use serde_json::Value;

/// 官方 CLI 默认 base（中国站）。
pub const DEFAULT_BASE: &str = "https://api.kimi.com/coding/v1";
/// 官方 CLI 国际站 base（`GLOBAL_KIMI_CODE_BASE_URL`）。
pub const GLOBAL_BASE: &str = "https://api.kimi.ai/coding/v1";
/// host 白名单（精确匹配，不用子串判定）。
const ALLOWED_HOSTS: [&str; 2] = ["api.kimi.com", "api.kimi.ai"];
/// 证据标注（E2：官方 CLI 源码中的只读接口，兼容可能随 CLI 变化）。
pub const EVIDENCE: &str =
    "kimi:coding_direct:E2:kimi-code@c9f01f07388abc2cdca40c9c5267f58c65086710";

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KimiCodeError {
    #[error("invalid base url: {0}")]
    InvalidBase(String),
    #[error("unrecognized Kimi Code host: {0}")]
    UnrecognizedHost(String),
    #[error("HTTP {status}")]
    Http { status: u16, code: Option<String> },
    #[error("schema changed: {0}")]
    SchemaChanged(String),
}

impl KimiCodeError {
    pub fn status(&self) -> SnapshotStatus {
        match self {
            Self::InvalidBase(_) => SnapshotStatus::Incompatible,
            Self::UnrecognizedHost(_) => SnapshotStatus::Unsupported,
            Self::Http { status, .. } => match status {
                401 | 403 => SnapshotStatus::AuthRequired,
                429 => SnapshotStatus::RateLimited,
                _ => SnapshotStatus::Unavailable,
            },
            Self::SchemaChanged(_) => SnapshotStatus::Incompatible,
        }
    }

    /// 直接喂给 `services::accounts::failure_record` 的信号。
    pub fn failure_signal(&self) -> FailureSignal {
        let mut signal = FailureSignal {
            status_hint: Some(self.status()),
            ..Default::default()
        };
        match self {
            Self::Http { status, code } => {
                signal.http_status = Some(*status);
                // 401 等错误体为 `{"error":{"message","type"}}`：`type` 是稳定的机器码，
                // 记录它便于区分“凭据无效”与其他 401；`message` 不落库（自由文本）。
                signal.business_code = code.clone();
            }
            Self::SchemaChanged(_) => signal.schema_changed = true,
            Self::UnrecognizedHost(_) => signal.unsupported = true,
            Self::InvalidBase(_) => {}
        }
        signal
    }
}

// ---------------------------------------------------------------------------
// 请求构造
// ---------------------------------------------------------------------------

/// 构造 `GET {base}/usages`。
///
/// base 只接受 **https + 白名单精确 host**（`api.kimi.com` / `api.kimi.ai`），
/// 不带显式端口；路径为 base 的路径段 + `/usages`。认证位置为 Bearer，
/// 秘密由服务层注入，规格中不含任何凭据。
pub fn build_request(base_url: &str) -> Result<RequestSpec, KimiCodeError> {
    let raw = base_url.trim().trim_end_matches('/');
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| KimiCodeError::InvalidBase(raw.to_owned()))?;
    if !scheme.eq_ignore_ascii_case("https") {
        return Err(KimiCodeError::InvalidBase(format!(
            "coding direct base must use https: {raw}"
        )));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(KimiCodeError::InvalidBase(raw.to_owned()));
    }
    let (host, port) = common::split_host_port(authority)
        .ok_or_else(|| KimiCodeError::InvalidBase(raw.to_owned()))?;
    if port.is_some() {
        return Err(KimiCodeError::InvalidBase(format!(
            "explicit port not supported: {raw}"
        )));
    }
    let host = host.to_ascii_lowercase();
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        return Err(KimiCodeError::UnrecognizedHost(host));
    }
    let base_path = rest[authority.len()..].trim_end_matches('/');
    let path = format!("{base_path}/usages");
    Ok(RequestSpec::https(host, path)
        .with_headers(vec![("Accept".to_owned(), "application/json".to_owned())])
        .with_auth(AuthPlacement::bearer()))
}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

/// `/usages` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KimiCodeReport {
    pub windows: Vec<QuotaWindow>,
    pub diagnostics: Vec<String>,
    pub evidence: String,
}

/// 窗口顺序固定：5h、7d、月总额、月代码；window_id / label / 周期与 B06 一致。
const WINDOWS: [(&str, &str, &str, PeriodKind, Option<u64>); 4] = [
    (
        "limit_5h",
        "5h",
        "5-hour rolling",
        PeriodKind::Rolling,
        Some(18_000),
    ),
    (
        "limit_7d",
        "7d",
        "7-day rolling",
        PeriodKind::Rolling,
        Some(604_800),
    ),
    (
        "limit_month_total",
        "month_total",
        "Monthly total (calendar)",
        PeriodKind::Calendar,
        None,
    ),
    (
        "limit_month_code",
        "month_code",
        "Monthly code (calendar)",
        PeriodKind::Calendar,
        None,
    ),
];

/// `status` / `body` → [`KimiCodeReport`]；错误按契约状态映射。
pub fn parse_usages(status: u16, body: &[u8]) -> Result<KimiCodeReport, KimiCodeError> {
    if status >= 400 {
        return Err(KimiCodeError::Http {
            status,
            code: error_type_of(body),
        });
    }
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| KimiCodeError::SchemaChanged(format!("body is not JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| KimiCodeError::SchemaChanged("body is not an object".into()))?;
    let usages = obj
        .get("usages")
        .and_then(Value::as_object)
        .ok_or_else(|| KimiCodeError::SchemaChanged("usages missing or not an object".into()))?;

    let mut diagnostics = Vec::new();
    let mut windows = Vec::new();
    for (field, key, label, period_kind, duration) in WINDOWS {
        let Some(item) = usages.get(field) else {
            continue;
        };
        let Some(entry) = item.as_object() else {
            diagnostics.push(format!("{field}_not_object"));
            continue;
        };
        let used_ratio = entry
            .get("used_ratio")
            .and_then(common::json_decimal)
            .ok_or_else(|| KimiCodeError::SchemaChanged(format!("{field}.used_ratio missing")))?;
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
        window.resets_at = parse_reset_time(entry, field, &mut diagnostics);
        window.used_ratio = Some(used);
        window.remaining_ratio = remaining;
        window.raw_ratio = raw;
        window.over_limit = over;
        window.quality = Quality::Reported;
        window.evidence_ref = Some(EVIDENCE.to_owned());
        windows.push(window);
    }
    for key in usages.keys() {
        if !WINDOWS.iter().any(|(field, ..)| field == key) {
            diagnostics.push(format!("unknown_usage_key:{key}"));
        }
    }

    Ok(KimiCodeReport {
        windows,
        diagnostics,
        evidence: format!("{EVIDENCE}:usages"),
    })
}

/// `reset_time` → RFC3339 校验（容忍有无小数秒），原样保留；非法值记诊断并保持未知。
fn parse_reset_time(
    entry: &serde_json::Map<String, Value>,
    field: &str,
    diagnostics: &mut Vec<String>,
) -> Option<String> {
    let text = entry
        .get("reset_time")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).is_err() {
        diagnostics.push(format!("{field}.reset_time_not_rfc3339"));
        return None;
    }
    Some(text.to_owned())
}

/// 错误体 `{"error":{"message","type"}}` 中的稳定机器码（`type`）。
fn error_type_of(body: &[u8]) -> Option<String> {
    let root: Value = serde_json::from_slice(body).ok()?;
    root.get("error")?
        .get("type")?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

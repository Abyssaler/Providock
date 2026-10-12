//! 账户查询服务层（T10 服务侧）：读账户与凭据 → 适配器构造请求 → 执行 HTTP → 解析 →
//! 成功写快照 / 失败写失败记录。
//!
//! 依据：
//! - `docs/specs/ACCOUNT_ADAPTERS.md` 第 5 节（调度与失败）、第 6 节（展示规则）、第 7 节（验收）；
//! - `docs/specs/SECURITY_PRIVACY.md` 第 4 节（网络规则）：准确 host allowlist、TLS 校验、
//!   有界超时/响应体、**跨 host 重定向不携带 Authorization**、诊断不记录完整响应；
//! - `docs/specs/DATA_CONTRACTS.md` 第 7/8 节（快照、窗口、用量指标）；
//! - `docs/ARCHITECTURE.md` 第 4 节：解析是纯函数，HTTP 与凭据注入在服务层。
//!
//! 边界：
//! - 秘密只在构造请求头的瞬间从配置的凭据库解封，随后立即 drop；任何返回值、错误与日志都不回显。
//! - 只做只读 GET；不重试有副作用请求（本服务只有只读查询，失败按契约记 stale/unavailable）。
//! - `query_codex` 为进程协议占位：B07 依赖官方 App Server 的 JSON-RPC（stdio/ws/unix），
//!   不属于简单 HTTP，本任务不接线，返回 `unsupported`。

use crate::adapters::accounts::common::{self, RequestSpec};
use crate::adapters::accounts::{deepseek, glm_plan, kimi_code, kimi_server, novita, openrouter};
use crate::services::accounts::{
    self, Account, AccountsError, BalanceEntry, CredentialType, FailureSignal, NewSnapshot,
    QuotaWindow, Snapshot, SnapshotStatus, UsageMetric,
};
use crate::services::redact;
use crate::storage::Database;
use crate::vault::{Vault, VaultError};
use serde::Serialize;
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;

/// 建立连接超时（秒）。
pub const CONNECT_TIMEOUT_SECONDS: u64 = 5;
/// 单次查询总超时（秒）。
pub const TOTAL_TIMEOUT_SECONDS: u64 = 15;
/// 响应体上限（1 MiB）：有界读取，超限即失败，不把超大响应读进内存。
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// 手工跟随重定向的最大跳数。
pub const MAX_REDIRECTS: usize = 5;

/// settings 表中 GLM / Kimi 的 base URL 覆盖键（用户配置的实例地址）。
pub const SETTING_GLM_BASE_URL: &str = "account.glm.base_url";
pub const SETTING_KIMI_BASE_URL: &str = "account.kimi_server.base_url";
/// Kimi Code 直连通道的 base URL 覆盖键（默认 `https://api.kimi.com/coding/v1`）。
pub const SETTING_KIMI_CODE_BASE_URL: &str = "account.kimi_code.base_url";

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum QueryError {
    #[error(transparent)]
    Accounts(#[from] AccountsError),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error("account not found: {0}")]
    AccountNotFound(String),
    #[error("account is soft-deleted: {0}")]
    AccountDeleted(String),
    #[error("unsupported platform for HTTP query: {0}")]
    UnsupportedPlatform(String),
    #[error("adapter request build failed: {0}")]
    Build(String),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("response body is not valid text")]
    NonTextBody,
}

pub type Result<T> = std::result::Result<T, QueryError>;

// ---------------------------------------------------------------------------
// 查询结果
// ---------------------------------------------------------------------------

/// 一次查询的结果：成功→新快照；失败→契约状态 + 保留的旧快照（stale）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    pub account_id: String,
    pub status: SnapshotStatus,
    /// 本次写入的快照 id（成功）或最近成功快照 id（失败时旧值保留）；都没有为 `None`。
    pub snapshot_id: Option<i64>,
    pub queried_at: String,
    pub provider_as_of: Option<String>,
    pub evidence: Option<String>,
    pub error_code: Option<String>,
    pub balances: Vec<BalanceEntry>,
    pub quota_windows: Vec<QuotaWindow>,
    pub usage_metrics: Vec<UsageMetric>,
    /// 展示用：最近成功快照是否已过期（失败后仍保留旧值）。
    pub stale: bool,
}

impl SnapshotOutcome {
    fn from_success(account_id: &str, snapshot_id: i64, draft: &NewSnapshot) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: SnapshotStatus::Ok,
            snapshot_id: Some(snapshot_id),
            queried_at: draft.queried_at.clone(),
            provider_as_of: draft.provider_as_of.clone(),
            evidence: draft.evidence.clone(),
            error_code: None,
            balances: draft.balances.clone(),
            quota_windows: draft.quota_windows.clone(),
            usage_metrics: draft.usage_metrics.clone(),
            stale: false,
        }
    }

    /// 失败：状态来自 [`accounts::failure_record`]；旧成功快照原样保留并标 stale。
    fn from_failure(
        account_id: &str,
        status: SnapshotStatus,
        queried_at: &str,
        signal: &FailureSignal,
        previous: Option<Snapshot>,
    ) -> Self {
        let (snapshot_id, provider_as_of, evidence, balances, quota_windows, usage_metrics) =
            match previous {
                Some(snapshot) => (
                    Some(snapshot.snapshot_id),
                    snapshot.provider_as_of,
                    snapshot.evidence,
                    snapshot.balances,
                    snapshot.quota_windows,
                    snapshot.usage_metrics,
                ),
                None => (None, None, None, Vec::new(), Vec::new(), Vec::new()),
            };
        let stale = snapshot_id.is_some();
        Self {
            account_id: account_id.to_owned(),
            status,
            snapshot_id,
            queried_at: queried_at.to_owned(),
            provider_as_of,
            evidence,
            error_code: signal.error_code(),
            balances,
            quota_windows,
            usage_metrics,
            stale,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP 传输抽象
// ---------------------------------------------------------------------------

/// 已解析、可直接发送的 HTTP 请求。
///
/// `auth_header` 携带**已解封的秘密**，因此 `Debug` 被手工实现为脱敏形态；
/// 秘密不会出现在任何错误或日志里。
#[derive(Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// `(头名, 完整头值)`，头值含秘密；跨 host 重定向时由传输层剥离。
    pub auth_header: Option<(String, String)>,
    pub timeout_seconds: u64,
    pub max_body_bytes: usize,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &redact::redact(&self.url))
            .field("headers", &self.headers)
            .field(
                "auth_header",
                &self
                    .auth_header
                    .as_ref()
                    .map(|(name, _)| format!("{name}: <redacted>")),
            )
            .field("timeout_seconds", &self.timeout_seconds)
            .field("max_body_bytes", &self.max_body_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("network timeout")]
    Timeout,
    #[error("network failure: {0}")]
    Network(String),
    #[error("blocked host/scheme: {0}")]
    BlockedHost(String),
    #[error("response body exceeds limit")]
    BodyTooLarge,
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("redirect without Location header")]
    RedirectWithoutLocation,
}

/// HTTP 传输抽象。生产实现走 rustls + 有界超时/响应体；测试可注入本地 mock。
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> std::result::Result<HttpResponse, TransportError>;
}

/// 生产传输：`reqwest::blocking`（rustls），关闭自动重定向并手工处理（见 [`ReqwestTransport::send`]）。
pub struct ReqwestTransport {
    client: OnceLock<reqwest::blocking::Client>,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: OnceLock::new(),
        }
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECONDS))
                .timeout(Duration::from_secs(TOTAL_TIMEOUT_SECONDS))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn map_reqwest_error(err: reqwest::Error) -> TransportError {
    if err.is_timeout() {
        return TransportError::Timeout;
    }
    // 错误文本脱敏：不含请求头，路径被匿名化，疑似密钥被剥离。
    TransportError::Network(redact::redact(&err.to_string()))
}

fn method_of(method: &str) -> std::result::Result<reqwest::Method, TransportError> {
    reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| TransportError::Network(format!("invalid method: {method}")))
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// 仅允许 HTTPS 外网，或 loopback 上的 HTTP（Kimi 本机 Server 为例外）。
pub fn ensure_url_allowed(url: &reqwest::Url) -> std::result::Result<(), TransportError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(TransportError::BlockedHost("url carries userinfo".into()));
    }
    let host = url.host_str().unwrap_or("");
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(host) => Ok(()),
        scheme => Err(TransportError::BlockedHost(format!(
            "scheme {scheme} to host {host} not allowed"
        ))),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    host == "localhost" || host == "::1" || host == "127.0.0.1" || host.starts_with("127.")
}

/// URL 的来源标识（host + 有效端口）：重定向跨来源时不携带凭证。
fn origin_authority_of(url: &reqwest::Url) -> Option<(String, u16)> {
    let host = url.host_str()?.to_owned();
    let port = url.port_or_known_default()?;
    Some((host, port))
}

impl HttpTransport for ReqwestTransport {
    /// 手工跟随重定向：`redirect::Policy::none()` 关闭自动跟随，这里逐跳解析 `Location`。
    ///
    /// 跨 host 重定向时**剥离 `Authorization`**（SECURITY_PRIVACY 第 4 节）；同 host 保留。
    /// 每一跳的目标都重新做 scheme/host 策略校验，防止重定向到非白名单或不安全目标。
    fn send(&self, request: &HttpRequest) -> std::result::Result<HttpResponse, TransportError> {
        let client = self.client();
        let mut url = reqwest::Url::parse(&request.url)
            .map_err(|e| TransportError::Network(format!("invalid url: {e}")))?;
        ensure_url_allowed(&url)?;
        // 以**首个请求的 authority（host+port）** 为基准：任何跳到不同 authority 的
        // 重定向都不再携带凭证（即使 host 相同、端口不同也算跨来源）。
        let origin_authority = origin_authority_of(&url);
        let mut carry_auth = true;
        let current_request = request.clone();

        for _ in 0..=MAX_REDIRECTS {
            let mut builder = client
                .request(method_of(&current_request.method)?, url.clone())
                .timeout(Duration::from_secs(current_request.timeout_seconds.max(1)));
            for (name, value) in &current_request.headers {
                builder = builder.header(name, value);
            }
            if carry_auth {
                if let Some((name, value)) = &current_request.auth_header {
                    builder = builder.header(name, value);
                }
            }
            let response = builder.send().map_err(map_reqwest_error)?;
            let status = response.status().as_u16();
            if is_redirect(status) {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(TransportError::RedirectWithoutLocation)?;
                let next = url.join(location).map_err(|e| {
                    TransportError::Network(format!(
                        "bad redirect target: {}",
                        redact::redact(&e.to_string())
                    ))
                })?;
                // 跨 authority（host 或端口不同）：不再携带 Authorization。
                if origin_authority_of(&next) != origin_authority {
                    carry_auth = false;
                }
                ensure_url_allowed(&next)?;
                url = next;
                continue;
            }
            let limit = current_request.max_body_bytes.saturating_add(1) as u64;
            let mut body = Vec::new();
            response
                .take(limit)
                .read_to_end(&mut body)
                .map_err(|e| TransportError::Network(redact::redact(&e.to_string())))?;
            if body.len() > current_request.max_body_bytes {
                return Err(TransportError::BodyTooLarge);
            }
            return Ok(HttpResponse { status, body });
        }
        Err(TransportError::TooManyRedirects)
    }
}

// ---------------------------------------------------------------------------
// 账户 → 适配器计划
// ---------------------------------------------------------------------------

/// 一次查询要调用的适配器与端点。
enum AdapterPlan {
    Deepseek,
    OpenRouter(openrouter::Endpoint),
    Novita,
    Glm {
        base: String,
    },
    /// B06：本机 Kimi Code Server（loopback）。
    Kimi {
        base: String,
    },
    /// B-04：Kimi Code Coding Key 直连 `GET {base}/usages`（官方 CLI 同一通道）。
    KimiCode {
        base: String,
    },
    /// B07：进程协议，不在本 HTTP 服务接线范围内。
    Codex,
}

fn setting_get(db: &Database, key: &str) -> Result<Option<String>> {
    let value: Option<String> = db
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .ok();
    Ok(value.filter(|v| !v.trim().is_empty()))
}

fn glm_default_base(region: Option<&str>) -> String {
    match region.unwrap_or("global").to_ascii_lowercase().as_str() {
        "cn" | "china" | "zh" | "zh-cn" => format!("https://{}", glm_plan::GlmRegion::China.host()),
        _ => format!("https://{}", glm_plan::GlmRegion::Global.host()),
    }
}

fn resolve_plan(
    db: &Database,
    account: &Account,
    credential_type: Option<CredentialType>,
) -> Result<AdapterPlan> {
    let platform = account.platform.to_ascii_lowercase();
    match platform.as_str() {
        "deepseek" => Ok(AdapterPlan::Deepseek),
        "openrouter" => {
            // 管理密钥走账户 Credits（B03），否则查当前推理 Key（B02）。
            let endpoint = match credential_type {
                Some(CredentialType::ManagementKey) => openrouter::Endpoint::Credits,
                _ => openrouter::Endpoint::CurrentKey,
            };
            Ok(AdapterPlan::OpenRouter(endpoint))
        }
        "novita" => Ok(AdapterPlan::Novita),
        "glm" | "glm_plan" | "zhipu" | "bigmodel" | "z.ai" => {
            let base = setting_get(db, SETTING_GLM_BASE_URL)?
                .unwrap_or_else(|| glm_default_base(account.region.as_deref()));
            Ok(AdapterPlan::Glm { base })
        }
        "kimi" | "kimi_server" | "kimi_code_server" => {
            // 凭据类型决定通道：Coding Key（api_key）走官方 CLI 的直连 usages 端点；
            // 本机 Server 令牌（或历史账户无凭据类型）保持 loopback B06。
            match credential_type {
                Some(CredentialType::ApiKey) => {
                    let base = setting_get(db, SETTING_KIMI_CODE_BASE_URL)?
                        .unwrap_or_else(|| kimi_code::DEFAULT_BASE.to_owned());
                    Ok(AdapterPlan::KimiCode { base })
                }
                _ => {
                    let base = setting_get(db, SETTING_KIMI_BASE_URL)?
                        .unwrap_or_else(|| kimi_server::DEFAULT_BASE.to_owned());
                    Ok(AdapterPlan::Kimi { base })
                }
            }
        }
        "codex" | "codex_appserver" => Ok(AdapterPlan::Codex),
        other => Err(QueryError::UnsupportedPlatform(other.to_owned())),
    }
}

fn build_spec(plan: &AdapterPlan) -> Result<RequestSpec> {
    let build = |e: String| QueryError::Build(e);
    match plan {
        AdapterPlan::Deepseek => deepseek::build_request(&deepseek::RequestConfig::canonical(
            deepseek::CANONICAL_HOST,
        ))
        .map_err(|e| build(e.to_string())),
        AdapterPlan::OpenRouter(endpoint) => {
            openrouter::build_request(&openrouter::RequestConfig::canonical(*endpoint))
                .map_err(|e| build(e.to_string()))
        }
        AdapterPlan::Novita => {
            novita::build_request(&novita::RequestConfig::canonical(novita::CANONICAL_HOST))
                .map_err(|e| build(e.to_string()))
        }
        AdapterPlan::Glm { base } => {
            glm_plan::build_request(base, glm_plan::GlmEndpoint::QuotaLimit, None)
                .map_err(|e| build(e.to_string()))
        }
        AdapterPlan::Kimi { base } => {
            kimi_server::build_request(base, kimi_server::KimiEndpoint::OauthUsage)
                .map_err(|e| build(e.to_string()))
        }
        AdapterPlan::KimiCode { base } => {
            kimi_code::build_request(base).map_err(|e| build(e.to_string()))
        }
        AdapterPlan::Codex => Err(QueryError::UnsupportedPlatform("codex".into())),
    }
}

/// 测试注入：把请求的 origin 换成 `base_override`，保留适配器决定的 path。
///
/// 这是「host 校验可注入点」：真实运行 `base_override = None` 时用适配器给出的精确
/// `https://<host><path>`；测试把非白名单外网 host 指向本地 127.0.0.1 mock，
/// 由传输层的「loopback http 例外」放行。
fn resolve_url(spec: &RequestSpec, base_override: Option<&str>) -> String {
    match base_override {
        None => spec.url(),
        Some(base) => format!("{}{}", base.trim_end_matches('/'), spec.path),
    }
}

// ---------------------------------------------------------------------------
// 解析分发
// ---------------------------------------------------------------------------

struct ParsedPayload {
    balances: Vec<BalanceEntry>,
    quota_windows: Vec<QuotaWindow>,
    provider_as_of: Option<String>,
    evidence: Option<String>,
}

enum ParseOutcome {
    Ok(ParsedPayload),
    Fail(FailureSignal),
}

fn split(signal: common::ParsedAccountResponse) -> ParseOutcome {
    match signal {
        common::ParsedAccountResponse::Success {
            balances,
            quota_windows,
            provider_as_of,
        } => ParseOutcome::Ok(ParsedPayload {
            balances,
            quota_windows,
            provider_as_of,
            evidence: None,
        }),
        common::ParsedAccountResponse::Failure(failure) => ParseOutcome::Fail(failure),
    }
}

fn parse_payload(plan: &AdapterPlan, status: u16, body: &[u8]) -> ParseOutcome {
    match plan {
        AdapterPlan::Deepseek | AdapterPlan::OpenRouter(_) | AdapterPlan::Novita => {
            let Ok(text) = std::str::from_utf8(body) else {
                return ParseOutcome::Fail(FailureSignal {
                    schema_changed: true,
                    ..Default::default()
                });
            };
            split(match plan {
                AdapterPlan::Deepseek => deepseek::parse_response(status, text),
                AdapterPlan::OpenRouter(endpoint) => {
                    openrouter::parse_response(*endpoint, status, text)
                }
                AdapterPlan::Novita => novita::parse_response(status, text),
                _ => unreachable!(),
            })
        }
        AdapterPlan::Glm { .. } => {
            match glm_plan::parse_response(glm_plan::GlmEndpoint::QuotaLimit, status, body) {
                Ok(report) => ParseOutcome::Ok(ParsedPayload {
                    balances: Vec::new(),
                    quota_windows: report.windows,
                    provider_as_of: None,
                    evidence: Some(report.evidence),
                }),
                Err(error) => ParseOutcome::Fail(error.failure_signal()),
            }
        }
        AdapterPlan::Kimi { .. } => match kimi_server::parse_oauth_usage(status, body) {
            Ok(report) => ParseOutcome::Ok(ParsedPayload {
                balances: report.balances,
                quota_windows: report.windows,
                provider_as_of: None,
                evidence: Some(report.evidence),
            }),
            Err(error) => ParseOutcome::Fail(error.failure_signal()),
        },
        AdapterPlan::KimiCode { .. } => match kimi_code::parse_usages(status, body) {
            Ok(report) => ParseOutcome::Ok(ParsedPayload {
                balances: Vec::new(),
                quota_windows: report.windows,
                provider_as_of: None,
                evidence: Some(report.evidence),
            }),
            Err(error) => ParseOutcome::Fail(error.failure_signal()),
        },
        // Codex 在 query_account_via 早期返回，不会走到这里。
        AdapterPlan::Codex => ParseOutcome::Fail(FailureSignal {
            unsupported: true,
            status_hint: Some(SnapshotStatus::Unsupported),
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

/// 执行一次账户查询（生产传输 + 适配器精确 host）。
pub fn query_account(
    db: &mut Database,
    vault: &dyn Vault,
    account_id: &str,
) -> Result<SnapshotOutcome> {
    let transport = ReqwestTransport::new();
    query_account_via(db, vault, account_id, &transport, None)
}

/// 可注入传输与 base 覆盖的查询入口（测试与未来的用户配置实例使用）。
pub fn query_account_via(
    db: &mut Database,
    vault: &dyn Vault,
    account_id: &str,
    transport: &dyn HttpTransport,
    base_override: Option<&str>,
) -> Result<SnapshotOutcome> {
    let prepared = prepare_query(db, account_id)?;
    let outcome = execute_query(&prepared, vault, transport, base_override)?;
    finish_query(db, &prepared, outcome)
}

struct PreparedQuery {
    account: Account,
    credential: Option<accounts::ActiveCredential>,
    plan: AdapterPlan,
    queried_at: String,
}

fn prepare_query(db: &Database, account_id: &str) -> Result<PreparedQuery> {
    let account = accounts::account_get(db, account_id)?
        .ok_or_else(|| QueryError::AccountNotFound(account_id.to_owned()))?;
    if account.is_deleted() {
        return Err(QueryError::AccountDeleted(account_id.to_owned()));
    }
    let credential = accounts::active_credential_ref(db, account_id)?;
    let plan = resolve_plan(db, &account, credential.as_ref().map(|c| c.credential_type))?;
    Ok(PreparedQuery {
        account,
        credential,
        plan,
        queried_at: accounts::now_rfc3339(),
    })
}

fn execute_query(
    prepared: &PreparedQuery,
    vault: &dyn Vault,
    transport: &dyn HttpTransport,
    base_override: Option<&str>,
) -> Result<ParseOutcome> {
    if matches!(prepared.plan, AdapterPlan::Codex) {
        return Ok(ParseOutcome::Fail(FailureSignal {
            unsupported: true,
            ..Default::default()
        }));
    }
    let spec = build_spec(&prepared.plan)?;
    let Some(credential) = &prepared.credential else {
        return Ok(ParseOutcome::Fail(FailureSignal {
            status_hint: Some(SnapshotStatus::AuthRequired),
            ..Default::default()
        }));
    };
    let secret = match vault.get(&credential.vault_ref) {
        Ok(secret) => secret,
        Err(VaultError::NotFound) => {
            return Ok(ParseOutcome::Fail(FailureSignal {
                status_hint: Some(SnapshotStatus::AuthRequired),
                ..Default::default()
            }))
        }
        Err(other) => return Err(QueryError::Vault(other)),
    };
    let auth_header = spec
        .auth
        .as_ref()
        .filter(|a| !a.is_server_held())
        .map(|auth| (auth.header.clone(), auth.header_value(secret.expose())));
    let request = HttpRequest {
        method: spec.method.clone(),
        url: resolve_url(&spec, base_override),
        headers: spec.headers.clone(),
        auth_header,
        timeout_seconds: spec.timeout_hint_seconds.clamp(1, TOTAL_TIMEOUT_SECONDS),
        max_body_bytes: MAX_BODY_BYTES,
    };
    drop(secret);
    Ok(match transport.send(&request) {
        Ok(response) => parse_payload(&prepared.plan, response.status, &response.body),
        Err(_) => ParseOutcome::Fail(FailureSignal {
            transport_failed: true,
            status_hint: Some(SnapshotStatus::Unavailable),
            ..Default::default()
        }),
    })
}

fn finish_query(
    db: &mut Database,
    prepared: &PreparedQuery,
    outcome: ParseOutcome,
) -> Result<SnapshotOutcome> {
    let account_id = &prepared.account.account_id;
    if matches!(prepared.plan, AdapterPlan::Codex) {
        return Ok(codex_unsupported(account_id));
    }
    match outcome {
        ParseOutcome::Ok(payload) => {
            let draft = NewSnapshot {
                account_id: account_id.clone(),
                credential_id: prepared
                    .credential
                    .as_ref()
                    .map(|c| c.credential_id.clone()),
                queried_at: prepared.queried_at.clone(),
                provider_as_of: payload.provider_as_of,
                evidence: payload.evidence,
                expires_at: None,
                balances: payload.balances,
                quota_windows: payload.quota_windows,
                usage_metrics: Vec::new(),
            };
            let snapshot_id = accounts::snapshot_write(db, &draft)?;
            Ok(SnapshotOutcome::from_success(
                account_id,
                snapshot_id,
                &draft,
            ))
        }
        ParseOutcome::Fail(signal) => record_failure(
            db,
            account_id,
            prepared
                .credential
                .as_ref()
                .map(|c| c.credential_id.as_str()),
            &prepared.queried_at,
            &signal,
        ),
    }
}

/// 生产共享入口：网络与凭据读取不持有数据库锁。完成时校验账户/凭据仍有效，
/// 避免删除、换凭据或编辑账户期间把旧请求结果写回。
pub fn query_account_shared(
    db: &std::sync::Mutex<Database>,
    vault: &dyn Vault,
    account_id: &str,
) -> Result<SnapshotOutcome> {
    query_account_shared_via(db, vault, account_id, &ReqwestTransport::new())
}

fn query_account_shared_via(
    db: &std::sync::Mutex<Database>,
    vault: &dyn Vault,
    account_id: &str,
    transport: &dyn HttpTransport,
) -> Result<SnapshotOutcome> {
    type QueryLocks = std::sync::Mutex<
        std::collections::HashMap<(usize, String), std::sync::Weak<std::sync::Mutex<()>>>,
    >;
    static LOCKS: OnceLock<QueryLocks> = OnceLock::new();
    let gate = {
        let mut locks = LOCKS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| QueryError::Build("query lock unavailable".into()))?;
        locks.retain(|_, gate| gate.strong_count() > 0);
        let key = (db as *const _ as usize, account_id.to_owned());
        match locks.get(&key).and_then(std::sync::Weak::upgrade) {
            Some(gate) => gate,
            None => {
                let gate = std::sync::Arc::new(std::sync::Mutex::new(()));
                locks.insert(key, std::sync::Arc::downgrade(&gate));
                gate
            }
        }
    };
    let _in_flight = gate
        .try_lock()
        .map_err(|_| QueryError::Build("account query already in progress".into()))?;
    let prepared = {
        let db = db
            .lock()
            .map_err(|_| QueryError::Build("database lock unavailable".into()))?;
        prepare_query(&db, account_id)?
    };
    let outcome = execute_query(&prepared, vault, transport, None)?;
    let mut db = db
        .lock()
        .map_err(|_| QueryError::Build("database lock unavailable".into()))?;
    let current = accounts::account_get(&db, account_id)?
        .ok_or_else(|| QueryError::AccountNotFound(account_id.into()))?;
    if current.is_deleted() {
        return Err(QueryError::AccountDeleted(account_id.into()));
    }
    if current != prepared.account
        || accounts::active_credential_ref(&db, account_id)? != prepared.credential
    {
        return Err(QueryError::Build(
            "account changed during query; refresh again".into(),
        ));
    }
    finish_query(&mut db, &prepared, outcome)
}

fn record_failure(
    db: &Database,
    account_id: &str,
    credential_id: Option<&str>,
    queried_at: &str,
    signal: &FailureSignal,
) -> Result<SnapshotOutcome> {
    let status = accounts::failure_record(db, account_id, credential_id, queried_at, signal)?;
    let previous = accounts::snapshot_latest(db, account_id)?.map(|(snapshot, _)| snapshot);
    Ok(SnapshotOutcome::from_failure(
        account_id, status, queried_at, signal, previous,
    ))
}

/// B07 Codex 官方 App Server 查询占位。
///
/// App Server 是 **JSON-RPC 进程协议**（默认 stdio，另有 `ws://` / `unix://`），不是简单
/// HTTP 端点；`account/rateLimits/read`、`account/usage/read` 需先 `initialize` 协商能力，
/// 且认证由对端持有。本任务的 HTTP 查询服务只覆盖 B01–B06，Codex 留占位：
/// 返回 `unsupported`（能力缺失，不是暂时失败），由 UI 显示“平台暂未提供查询能力”。
pub fn query_codex(
    _db: &mut Database,
    _vault: &dyn Vault,
    account_id: &str,
) -> Result<SnapshotOutcome> {
    Ok(codex_unsupported(account_id))
}

fn codex_unsupported(account_id: &str) -> SnapshotOutcome {
    SnapshotOutcome {
        account_id: account_id.to_owned(),
        status: SnapshotStatus::Unsupported,
        snapshot_id: None,
        queried_at: accounts::now_rfc3339(),
        provider_as_of: None,
        evidence: Some("codex:appserver:process_protocol_not_wired".to_owned()),
        error_code: Some("unsupported".to_owned()),
        balances: Vec::new(),
        quota_windows: Vec::new(),
        usage_metrics: Vec::new(),
        stale: false,
    }
}

/// 供上层构造鉴权头做单元测试：不暴露任何秘密来源。
#[cfg(test)]
fn auth_header_for(
    placement: &crate::adapters::accounts::common::AuthPlacement,
    secret: &crate::vault::SecretString,
) -> (String, String) {
    (
        placement.header.clone(),
        placement.header_value(secret.expose()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::accounts::common::AuthPlacement;
    use crate::services::accounts::{
        account_create, credential_save, snapshot_latest, ConfigKind, NewAccount,
    };
    use crate::vault::{MemoryVault, SecretString};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    const SECRET: &str = "sk-synthetic-account-query";

    #[derive(Debug, Clone)]
    struct Captured {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
    }

    impl Captured {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    /// 本地 mock HTTP 服务：对每个预期响应接受一次连接，记录请求并写回响应。
    struct MockServer {
        base_url: String,
        captured: Arc<Mutex<Vec<Captured>>>,
        handle: Option<JoinHandle<()>>,
    }

    impl MockServer {
        fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
            let addr = listener.local_addr().unwrap();
            let captured = Arc::new(Mutex::new(Vec::new()));
            let store = Arc::clone(&captured);
            let handle = std::thread::spawn(move || {
                for response in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        break;
                    };
                    let request = read_request(&mut stream);
                    store.lock().unwrap().push(request);
                    let _ = write_response(&mut stream, &response);
                }
            });
            Self {
                base_url: format!("http://{}", addr),
                captured,
                handle: Some(handle),
            }
        }

        fn requests(&self) -> Vec<Captured> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            // 连接本地 listener 以唤醒 accept 线程，使其退出。
            let _ = TcpStream::connect(self.base_url.trim_start_matches("http://"));
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    struct MockResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl MockResponse {
        fn ok(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                body: body.into(),
            }
        }

        fn status(status: u16, body: impl Into<String>) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: body.into(),
            }
        }

        fn redirect(status: u16, location: impl Into<String>) -> Self {
            Self {
                status,
                headers: vec![("Location".into(), location.into())],
                body: String::new(),
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> Captured {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines = text.lines();
        let request_line = lines.next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_owned();
        let path = parts.next().unwrap_or("").to_owned();
        let headers = lines
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                line.split_once(':')
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            })
            .collect();
        Captured {
            method,
            path,
            headers,
        }
    }

    fn write_response(stream: &mut TcpStream, response: &MockResponse) -> std::io::Result<()> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            response.status,
            reason(response.status),
            response.body.len()
        );
        for (name, value) in &response.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str("\r\n");
        out.push_str(&response.body);
        stream.write_all(out.as_bytes())?;
        stream.flush()
    }

    fn reason(status: u16) -> &'static str {
        match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "Status",
        }
    }

    fn save_credential(
        db: &Database,
        vault: &MemoryVault,
        account_id: &str,
        kind: CredentialType,
    ) -> String {
        credential_save(
            db,
            vault,
            account_id,
            kind,
            None,
            &SecretString::new(SECRET),
        )
        .unwrap()
        .credential_id
    }

    // ---- 成功写快照 --------------------------------------------------------

    #[test]
    fn deepseek_success_writes_snapshot() {
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"12.34","granted_balance":"2.00","topped_up_balance":"10.34"}]}"#,
        )]);
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("deepseek", "DS", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::Ok);
        assert!(outcome.snapshot_id.is_some());
        assert!(!outcome.stale);
        assert_eq!(outcome.balances.len(), 3);
        assert_eq!(outcome.balances[0].amount, "12.34");
        // 真实落库
        let (snapshot, stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert!(!stale);
        assert_eq!(snapshot.balances.len(), 3);
        assert!(snapshot.credential_id.is_some());
        // 请求确实带上了 Authorization（mock 收到 Bearer 头）。
        let requests = server.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/user/balance");
        assert!(requests[0]
            .header("authorization")
            .unwrap()
            .starts_with("Bearer "));
    }

    #[test]
    fn kimi_loopback_success_via_settings_base() {
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"code":0,"data":{"kind":"ok","quota":{"usages":{"limit5h":{"usedRatio":"0.25"}},"extraUsage":null}}}"#,
        )]);
        let mut db = Database::open_in_memory().unwrap();
        let account = account_create(
            &db,
            &NewAccount::new("kimi_server", "Kimi", ConfigKind::CodingPlan),
        )
        .unwrap();
        let vault = MemoryVault::new();
        save_credential(
            &db,
            &vault,
            &account.account_id,
            CredentialType::LocalServerToken,
        );
        // 用户配置的 loopback 实例（这里指向本地 mock）。
        db.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![SETTING_KIMI_BASE_URL, server.base_url],
            )
            .unwrap();

        let outcome = query_account(&mut db, &vault, &account.account_id).unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Ok);
        assert_eq!(outcome.quota_windows.len(), 1);
        assert_eq!(outcome.quota_windows[0].used_ratio.as_deref(), Some("0.25"));
        let requests = server.requests();
        assert_eq!(requests[0].path, "/api/v1/oauth/usage");
        assert!(requests[0]
            .header("authorization")
            .unwrap()
            .starts_with("Bearer "));
    }

    // ---- Kimi 凭据类型路由（B06 loopback vs B-04 直连） ---------------------

    #[test]
    fn kimi_credential_type_selects_channel() {
        let db = Database::open_in_memory().unwrap();
        let account = account_create(
            &db,
            &NewAccount::new("kimi", "Kimi", ConfigKind::CodingPlan),
        )
        .unwrap();

        // 本机 Server 令牌 / 无凭据 → loopback B06 默认地址。
        for credential_type in [Some(CredentialType::LocalServerToken), None] {
            match resolve_plan(&db, &account, credential_type).unwrap() {
                AdapterPlan::Kimi { base } => {
                    assert_eq!(base, crate::adapters::accounts::kimi_server::DEFAULT_BASE)
                }
                other => panic!("expected loopback plan, got {}", plan_name(&other)),
            }
        }

        // Coding Key（api_key）→ 直连 usages 通道，默认官方 base。
        match resolve_plan(&db, &account, Some(CredentialType::ApiKey)).unwrap() {
            AdapterPlan::KimiCode { base } => {
                assert_eq!(base, crate::adapters::accounts::kimi_code::DEFAULT_BASE)
            }
            other => panic!("expected direct plan, got {}", plan_name(&other)),
        }

        // 两个 base 覆盖键互不争抢：server 键不影响直连，直连键不影响 loopback。
        db.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![SETTING_KIMI_BASE_URL, "http://127.0.0.1:9000"],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![SETTING_KIMI_CODE_BASE_URL, "https://api.kimi.ai/coding/v1"],
            )
            .unwrap();
        match resolve_plan(&db, &account, Some(CredentialType::ApiKey)).unwrap() {
            AdapterPlan::KimiCode { base } => assert_eq!(base, "https://api.kimi.ai/coding/v1"),
            other => panic!("expected direct override, got {}", plan_name(&other)),
        }
        match resolve_plan(&db, &account, Some(CredentialType::LocalServerToken)).unwrap() {
            AdapterPlan::Kimi { base } => assert_eq!(base, "http://127.0.0.1:9000"),
            other => panic!("expected loopback override, got {}", plan_name(&other)),
        }
    }

    fn plan_name(plan: &AdapterPlan) -> &'static str {
        match plan {
            AdapterPlan::Deepseek => "deepseek",
            AdapterPlan::OpenRouter(_) => "openrouter",
            AdapterPlan::Novita => "novita",
            AdapterPlan::Glm { .. } => "glm",
            AdapterPlan::Kimi { .. } => "kimi_loopback",
            AdapterPlan::KimiCode { .. } => "kimi_code_direct",
            AdapterPlan::Codex => "codex",
        }
    }

    #[test]
    fn kimi_api_key_queries_direct_usages_channel() {
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"usages":{"limit_5h":{"used_ratio":0.25,"reset_time":"2026-10-12T09:30:00Z"},
                          "limit_7d":{"used_ratio":0.6}}}"#,
        )]);
        let mut db = Database::open_in_memory().unwrap();
        let account = account_create(
            &db,
            &NewAccount::new("kimi", "Kimi", ConfigKind::CodingPlan),
        )
        .unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::Ok);
        assert_eq!(outcome.quota_windows.len(), 2);
        assert_eq!(
            outcome.quota_windows[0].window_id,
            "kimi:coding_plan:account:5h"
        );
        assert_eq!(
            outcome.quota_windows[0].resets_at.as_deref(),
            Some("2026-10-12T09:30:00Z")
        );
        assert!(outcome
            .evidence
            .as_deref()
            .unwrap()
            .contains("kimi:coding_direct"));
        let requests = server.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/coding/v1/usages");
        assert!(requests[0]
            .header("authorization")
            .unwrap()
            .starts_with("Bearer "));
    }

    #[test]
    fn kimi_api_key_401_maps_auth_required() {
        let server = MockServer::start(vec![MockResponse::status(
            401,
            r#"{"error":{"message":"Invalid authentication credentials provided.","type":"invalid_authentication_error"}}"#,
        )]);
        let mut db = Database::open_in_memory().unwrap();
        let account = account_create(
            &db,
            &NewAccount::new("kimi", "Kimi", ConfigKind::CodingPlan),
        )
        .unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::AuthRequired);
        assert_eq!(
            outcome.error_code.as_deref(),
            Some("invalid_authentication_error")
        );
        assert!(outcome.snapshot_id.is_none());
    }

    // ---- GLM 无套餐 → no_plan 落库 ------------------------------------------

    #[test]
    fn glm_no_plan_records_no_plan_status() {
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"code":500,"msg":"当前用户不存在coding plan","success":false}"#,
        )]);
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("glm", "GLM", ConfigKind::CodingPlan)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::NoPlan);
        assert_eq!(outcome.error_code.as_deref(), Some("500"));
        assert!(outcome.snapshot_id.is_none());
        // 失败行确实以 no_plan 落库，latest status 读回一致。
        assert_eq!(
            accounts::snapshot_latest_status(&db, &account.account_id).unwrap(),
            Some(SnapshotStatus::NoPlan)
        );
        let stored: String = db
            .conn()
            .query_row(
                "SELECT status FROM account_snapshots WHERE account_id = ?1",
                [&account.account_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "no_plan");
    }

    // ---- 失败映射 ----------------------------------------------------------

    fn prime_success(
        db: &mut Database,
        vault: &MemoryVault,
        account_id: &str,
        platform: &str,
    ) -> i64 {
        let server = MockServer::start(vec![MockResponse::ok(success_body(platform))]);
        let outcome = query_account_via(
            db,
            vault,
            account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Ok);
        outcome.snapshot_id.unwrap()
    }

    fn success_body(platform: &str) -> String {
        match platform {
            "deepseek" => r#"{"balance_infos":[{"currency":"USD","total_balance":"5.00","granted_balance":"0.00","topped_up_balance":"5.00"}]}"#.to_owned(),
            "kimi_server" => r#"{"code":0,"data":{"kind":"ok","quota":{"usages":{"limit5h":{"usedRatio":"0.1"}}}}}"#.to_owned(),
            other => panic!("no fixture for {other}"),
        }
    }

    #[test]
    fn unauthorized_keeps_old_snapshot_and_marks_stale() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("deepseek", "DS", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);
        let first_id = prime_success(&mut db, &vault, &account.account_id, "deepseek");

        let server = MockServer::start(vec![MockResponse::status(401, "{}")]);
        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::AuthRequired);
        assert!(outcome.stale, "旧快照保留并标 stale");
        assert_eq!(outcome.snapshot_id, Some(first_id));
        assert_eq!(outcome.balances.len(), 3, "旧值保留");
        // DB：最近成功快照未被覆盖，且失败行使其 stale。
        let (snapshot, stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert_eq!(snapshot.snapshot_id, first_id);
        assert!(stale);
    }

    #[test]
    fn rate_limited_maps_to_rate_limited() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("novita", "NV", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);
        let server = MockServer::start(vec![MockResponse::status(429, "{}")]);
        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::RateLimited);
        assert!(outcome.snapshot_id.is_none(), "没有成功快照");
    }

    #[test]
    fn http_200_business_error_is_a_failure() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("novita", "NV", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"code":40101,"message":"bad key"}"#,
        )]);
        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Unavailable);
        assert_eq!(outcome.error_code.as_deref(), Some("40101"));
    }

    #[test]
    fn missing_credential_records_auth_required_without_network() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("deepseek", "DS", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some("http://127.0.0.1:1"),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::AuthRequired);
        assert!(outcome.snapshot_id.is_none());
    }

    #[test]
    fn codex_query_is_unsupported_placeholder() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("codex", "CX", ConfigKind::CodingPlan)).unwrap();
        let vault = MemoryVault::new();
        let outcome = query_codex(&mut db, &vault, &account.account_id).unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Unsupported);
        assert!(outcome.snapshot_id.is_none());
    }

    // ---- 跨 host 重定向不带 Authorization ---------------------------------

    #[test]
    fn cross_host_redirect_strips_authorization() {
        // 目标 listener（B）：记录收到的请求。
        let target = MockServer::start(vec![MockResponse::ok(
            r#"{"balance_infos":[{"currency":"USD","total_balance":"1.00","granted_balance":"0.00","topped_up_balance":"1.00"}]}"#,
        )]);
        // 源 listener（A）：302 → 目标。
        let redirect = MockServer::start(vec![MockResponse::redirect(
            302,
            format!("{}/user/balance", target.base_url),
        )]);

        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("deepseek", "DS", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&redirect.base_url),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Ok);

        // 源请求带 Authorization；跨 host 重定向后的目标请求必须剥离。
        let first = redirect.requests();
        assert!(first[0].header("authorization").is_some(), "首跳带凭证");
        let second = target.requests();
        assert!(
            second[0].header("authorization").is_none(),
            "跨 host 重定向不得携带 Authorization"
        );
    }

    #[test]
    fn same_host_redirect_keeps_authorization() {
        // 同一 mock 进程处理两跳（redirect → 实际响应）。
        let server = MockServer::start(vec![
            MockResponse::redirect(302, "/real"),
            MockResponse::ok(r#"{"data":{"usage":1.0}}"#),
        ]);
        let mut db = Database::open_in_memory().unwrap();
        let account = account_create(
            &db,
            &NewAccount::new("openrouter", "OR", ConfigKind::ApiPayg),
        )
        .unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);

        let outcome = query_account_via(
            &mut db,
            &vault,
            &account.account_id,
            &ReqwestTransport::new(),
            Some(&server.base_url),
        )
        .unwrap();
        assert_eq!(outcome.status, SnapshotStatus::Ok);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].header("authorization").is_some());
        assert!(
            requests[1].header("authorization").is_some(),
            "同 host 重定向保留凭证"
        );
    }

    // ---- host/策略校验（纯函数） ------------------------------------------

    #[test]
    fn url_policy_allows_https_and_loopback_http_only() {
        assert!(ensure_url_allowed(
            &reqwest::Url::parse("https://api.deepseek.com/user/balance").unwrap()
        )
        .is_ok());
        assert!(
            ensure_url_allowed(&reqwest::Url::parse("http://127.0.0.1:8080/x").unwrap()).is_ok()
        );
        assert!(ensure_url_allowed(&reqwest::Url::parse("http://localhost:1/x").unwrap()).is_ok());
        assert!(ensure_url_allowed(&reqwest::Url::parse("http://[::1]:1/x").unwrap()).is_ok());
        // 外网 http 被拒。
        assert!(matches!(
            ensure_url_allowed(&reqwest::Url::parse("http://api.deepseek.com/x").unwrap()),
            Err(TransportError::BlockedHost(_))
        ));
        // 带 userinfo 的 URL 被拒。
        assert!(matches!(
            ensure_url_allowed(&reqwest::Url::parse("https://user:pw@api.deepseek.com/x").unwrap()),
            Err(TransportError::BlockedHost(_))
        ));
    }

    #[test]
    fn request_debug_does_not_leak_secret() {
        let request = HttpRequest {
            method: "GET".into(),
            url: "https://api.deepseek.com/user/balance".into(),
            headers: vec![],
            auth_header: Some(auth_header_for(
                &AuthPlacement::bearer(),
                &SecretString::new(SECRET),
            )),
            timeout_seconds: 15,
            max_body_bytes: MAX_BODY_BYTES,
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains(SECRET), "Debug 不得泄露秘密: {debug}");
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn unsupported_platform_is_rejected() {
        let mut db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("mystery", "X", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        assert!(matches!(
            query_account_via(
                &mut db,
                &vault,
                &account.account_id,
                &ReqwestTransport::new(),
                None
            ),
            Err(QueryError::UnsupportedPlatform(_))
        ));
    }

    /// IPC / 调度器都在 `spawn_blocking` 里跑查询：验证阻塞传输在该执行环境下可正常工作
    /// （不 panic、不在异步工作线程上阻塞）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn query_runs_inside_spawn_blocking() {
        let server = MockServer::start(vec![MockResponse::ok(
            r#"{"balance_infos":[{"currency":"USD","total_balance":"3.00","granted_balance":"0.00","topped_up_balance":"3.00"}]}"#,
        )]);
        let base = server.base_url.clone();
        let db = Database::open_in_memory().unwrap();
        let account =
            account_create(&db, &NewAccount::new("deepseek", "DS", ConfigKind::ApiPayg)).unwrap();
        let vault = MemoryVault::new();
        save_credential(&db, &vault, &account.account_id, CredentialType::ApiKey);
        let account_id = account.account_id.clone();

        let outcome = tokio::task::spawn_blocking(move || {
            let mut db = db;
            query_account_via(
                &mut db,
                &vault,
                &account_id,
                &ReqwestTransport::new(),
                Some(&base),
            )
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(outcome.status, SnapshotStatus::Ok);
        assert_eq!(outcome.balances[0].amount, "3.00");
    }
    struct InspectTransport<'a> {
        db: &'a Mutex<Database>,
        account_id: &'a str,
        delete: bool,
    }
    impl HttpTransport for InspectTransport<'_> {
        fn send(
            &self,
            _request: &HttpRequest,
        ) -> std::result::Result<HttpResponse, TransportError> {
            let guard = self.db.try_lock().expect("HTTP held the database lock");
            if self.delete {
                guard.conn().execute("UPDATE accounts SET deleted_at = '2026-10-11T12:00:00Z' WHERE account_id = ?1", [self.account_id]).unwrap();
            }
            drop(guard);
            // Another refresh of this same account must not send a second request.
            let result =
                query_account_shared_via(self.db, &MemoryVault::new(), self.account_id, self);
            assert!(
                matches!(result, Err(QueryError::Build(message)) if message.contains("already in progress"))
            );
            Ok(HttpResponse { status: 200, body: br#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"12.34","granted_balance":"2.00","topped_up_balance":"10.34"}]}"#.to_vec() })
        }
    }

    #[test]
    fn shared_query_releases_database_lock_and_coalesces_concurrent_refreshes() {
        let db = Mutex::new(Database::open_in_memory().unwrap());
        let vault = MemoryVault::new();
        let account = {
            let guard = db.lock().unwrap();
            let account = account_create(
                &guard,
                &NewAccount::new("deepseek", "test", ConfigKind::ApiPayg),
            )
            .unwrap();
            save_credential(&guard, &vault, &account.account_id, CredentialType::ApiKey);
            account
        };
        let result = query_account_shared_via(
            &db,
            &vault,
            &account.account_id,
            &InspectTransport {
                db: &db,
                account_id: &account.account_id,
                delete: false,
            },
        )
        .unwrap();
        assert_eq!(result.status, SnapshotStatus::Ok);
        assert_eq!(
            db.lock()
                .unwrap()
                .conn()
                .query_row("SELECT COUNT(*) FROM account_snapshots", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn shared_query_does_not_write_back_after_account_deleted_during_http() {
        let db = Mutex::new(Database::open_in_memory().unwrap());
        let vault = MemoryVault::new();
        let account = {
            let guard = db.lock().unwrap();
            let account = account_create(
                &guard,
                &NewAccount::new("deepseek", "test", ConfigKind::ApiPayg),
            )
            .unwrap();
            save_credential(&guard, &vault, &account.account_id, CredentialType::ApiKey);
            account
        };
        let result = query_account_shared_via(
            &db,
            &vault,
            &account.account_id,
            &InspectTransport {
                db: &db,
                account_id: &account.account_id,
                delete: true,
            },
        );
        assert!(matches!(result, Err(QueryError::AccountDeleted(_))));
        assert_eq!(
            db.lock()
                .unwrap()
                .conn()
                .query_row("SELECT COUNT(*) FROM account_snapshots", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

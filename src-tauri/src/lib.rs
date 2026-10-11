// Tauri 命令壳与业务模块装配。
// domain（契约类型）/ storage（自有 DB）/ ingestion（事务游标与去重）为 T02；
// adapters（Kimi/Codex 纯解析器）+ services.aggregate + IPC 纵线为 T04。

pub mod adapters;
pub mod domain;
pub mod gateway;
pub mod ingestion;
pub mod safepath;
pub mod services;
pub mod storage;
pub mod vault;

use ingestion::scanner::{discover_sources, scan_source, SourceScanReport};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use storage::Database;
use tauri::Manager;
use vault::Vault;

pub struct AppState {
    db: Arc<Mutex<Database>>,
    vault: Arc<dyn Vault>,
    /// 账户查询调度器句柄：Drop/`shutdown` 时取消后台任务。
    scheduler: Mutex<Option<services::account_scheduler::SchedulerHandle>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    identifier: &'static str,
}

/// 最小联通性命令：前端据此区分桌面运行与浏览器预览。
#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        identifier: "io.providock.app",
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFilterDto {
    pub from: Option<String>,
    pub to: Option<String>,
    pub agent: Option<String>,
    pub stream_domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub source_id: String,
    pub agent: String,
    pub root_path: String,
    pub display_label: Option<String>,
    pub parser_version: String,
    pub compat_status: String,
    pub enabled: bool,
    pub last_sync_at: Option<String>,
    pub events: i64,
}

fn map_filter(f: UsageFilterDto) -> services::aggregate::UsageFilter {
    services::aggregate::UsageFilter {
        from: f.from,
        to: f.to,
        agent: f.agent,
        stream_domain: f.stream_domain,
    }
}

/// 触发一次全源扫描（后台线程；C03：重扫幂等，游标与事件同事务提交）。
#[tauri::command]
async fn sync_start(state: tauri::State<'_, AppState>) -> Result<Vec<SourceScanReport>, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let sources = discover_sources();
        let mut db = db.lock().map_err(|e| e.to_string())?;
        let mut reports = Vec::new();
        for src in &sources {
            match scan_source(&mut db, src) {
                Ok(r) => reports.push(r),
                Err(e) => reports.push(SourceScanReport {
                    source_id: src.source_id.clone(),
                    agent: src.agent.as_str().into(),
                    files_seen: 0,
                    files_scanned: 0,
                    files_unchanged: 0,
                    files_deferred: 0,
                    events_inserted: 0,
                    events_replaced: 0,
                    events_already_present: 0,
                    diagnostics: vec![format!("scan_failed: {e}")],
                }),
            }
        }
        Ok(reports)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sources_list(state: tauri::State<'_, AppState>) -> Result<Vec<SourceInfo>, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT s.source_id, s.agent, s.root_path, s.display_label, s.parser_version,
                        s.compat_status, s.enabled, s.last_sync_at,
                        (SELECT COUNT(*) FROM usage_events e WHERE e.source_id = s.source_id) AS events
                 FROM sources s ORDER BY s.agent",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceInfo {
                    source_id: r.get(0)?,
                    agent: r.get(1)?,
                    root_path: r.get(2)?,
                    display_label: r.get(3)?,
                    parser_version: r.get(4)?,
                    compat_status: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    last_sync_at: r.get(7)?,
                    events: r.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn usage_summary(
    state: tauri::State<'_, AppState>,
    filter: UsageFilterDto,
) -> Result<services::aggregate::UsageSummary, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        services::aggregate::usage_summary(&db, &map_filter(filter)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn usage_series(
    state: tauri::State<'_, AppState>,
    filter: UsageFilterDto,
) -> Result<Vec<(String, Option<String>)>, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        services::aggregate::usage_series_daily(&db, &map_filter(filter)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 账户 IPC（T10）：只收/返 DTO；秘密只在提交瞬间进入后端，响应绝不回显
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDto {
    pub credential_id: String,
    pub credential_type: String,
    pub mask: String,
    pub fingerprint: String,
    pub capabilities: Option<serde_json::Value>,
    pub created_at: String,
    pub retired_at: Option<String>,
}

impl From<services::accounts::CredentialRef> for CredentialDto {
    fn from(credential: services::accounts::CredentialRef) -> Self {
        Self {
            credential_id: credential.credential_id,
            credential_type: credential.credential_type.as_str().to_owned(),
            mask: credential.mask,
            fingerprint: credential.fingerprint,
            capabilities: credential.capabilities,
            created_at: credential.created_at,
            retired_at: credential.retired_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummaryDto {
    pub account_id: String,
    pub platform: String,
    pub region: Option<String>,
    pub display_name: String,
    pub config_kind: String,
    pub created_at: String,
    pub credentials: Vec<CredentialDto>,
    /// 最近成功快照（含余额、窗口、用量指标）；没有成功查询为 null。
    pub snapshot: Option<services::accounts::Snapshot>,
    /// 展示用：最近成功快照已过期（其后有失败查询）。
    pub stale: bool,
    /// 最近一行查询的状态（含失败），无任何查询为 null。
    pub latest_status: Option<services::accounts::SnapshotStatus>,
}

fn build_account_summary(db: &Database, account_id: &str) -> Result<AccountSummaryDto, String> {
    use services::accounts as acc;
    let account = acc::account_get(db, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("account not found: {account_id}"))?;
    let credentials = acc::credentials_list(db, account_id, true)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CredentialDto::from)
        .collect();
    let (snapshot, stale) = match acc::snapshot_latest(db, account_id).map_err(|e| e.to_string())? {
        Some((snapshot, stale)) => (Some(snapshot), stale),
        None => (None, false),
    };
    let latest_status = acc::snapshot_latest_status(db, account_id).map_err(|e| e.to_string())?;
    Ok(AccountSummaryDto {
        account_id: account.account_id,
        platform: account.platform,
        region: account.region,
        display_name: account.display_name,
        config_kind: account.config_kind.as_str().to_owned(),
        created_at: account.created_at,
        credentials,
        snapshot,
        stale,
        latest_status,
    })
}

#[tauri::command]
async fn accounts_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AccountSummaryDto>, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let accounts = services::accounts::accounts_list(&db, false).map_err(|e| e.to_string())?;
        accounts
            .into_iter()
            .map(|account| build_account_summary(&db, &account.account_id))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSaveInput {
    /// `None` = 新建账户；`Some` = 更新既有账户。
    pub account_id: Option<String>,
    pub platform: String,
    pub region: Option<String>,
    pub display_name: String,
    pub scope: Option<String>,
    pub config_kind: String,
    pub credential_type: Option<String>,
    /// 前端仅在提交表单瞬间传入；后端立即写入系统凭据库，响应不回显。
    pub secret: Option<String>,
    pub capabilities: Option<serde_json::Value>,
}

#[tauri::command]
async fn account_save(
    state: tauri::State<'_, AppState>,
    input: AccountSaveInput,
) -> Result<AccountSummaryDto, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        use services::accounts as acc;
        let db = db.lock().map_err(|e| e.to_string())?;
        let config_kind = acc::ConfigKind::parse(&input.config_kind)
            .ok_or_else(|| format!("unknown config_kind: {}", input.config_kind))?;

        let account_id = match input.account_id.clone() {
            Some(account_id) => {
                acc::account_update(
                    &db,
                    &account_id,
                    &acc::AccountPatch {
                        region: input.region.clone(),
                        display_name: Some(input.display_name.clone()),
                        scope: input.scope.clone(),
                        config_kind: Some(config_kind),
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?;
                account_id
            }
            None => {
                let draft = acc::NewAccount {
                    platform: input.platform.clone(),
                    region: input.region.clone(),
                    display_name: input.display_name.clone(),
                    vendor_user_id: None,
                    scope: input.scope.clone(),
                    config_kind,
                };
                acc::account_create(&db, &draft)
                    .map_err(|e| e.to_string())?
                    .account_id
            }
        };

        // 只有前端明确提交秘密时才写入凭据库；秘密在此作用域内即被清零释放。
        if let Some(secret) = input.secret.as_deref() {
            let credential_type = input
                .credential_type
                .as_deref()
                .and_then(acc::CredentialType::parse)
                .ok_or_else(|| "credential_type is required when saving a secret".to_owned())?;
            let secret = vault::SecretString::new(secret);
            acc::credential_save(
                &db,
                vault.as_ref(),
                &account_id,
                credential_type,
                input.capabilities.clone(),
                &secret,
            )
            .map_err(|e| e.to_string())?;
        }

        build_account_summary(&db, &account_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRotateInput {
    pub credential_id: String,
    pub secret: String,
    pub capabilities: Option<serde_json::Value>,
}

#[tauri::command]
async fn account_rotate_credential(
    state: tauri::State<'_, AppState>,
    input: AccountRotateInput,
) -> Result<CredentialDto, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        let mut db = db.lock().map_err(|e| e.to_string())?;
        let secret = vault::SecretString::new(input.secret);
        let rotated = services::accounts::credential_rotate(
            &mut db,
            vault.as_ref(),
            &input.credential_id,
            input.capabilities,
            &secret,
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(CredentialDto::from(rotated))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn account_refresh(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<services::account_query::SnapshotOutcome, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        let mut db = db.lock().map_err(|e| e.to_string())?;
        services::account_query::query_account(&mut db, vault.as_ref(), &account_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeleteInput {
    pub account_id: String,
    /// 默认保留历史快照；`false` 时连同快照一并删除。
    #[serde(default = "default_keep_history")]
    pub keep_history: bool,
}

fn default_keep_history() -> bool {
    true
}

#[tauri::command]
async fn account_delete(
    state: tauri::State<'_, AppState>,
    input: AccountDeleteInput,
) -> Result<services::cleanup::AccountDeleteResult, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        let mut db = db.lock().map_err(|e| e.to_string())?;
        services::cleanup::delete_account_fully(
            &mut db,
            vault.as_ref(),
            &input.account_id,
            input.keep_history,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 会话 IPC（T13）：列表只查索引元数据；正文默认关闭，开启后按源文件按需读取
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsFilterDto {
    pub agent: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// 只匹配索引元数据（session_id / 标题），不扫正文。
    pub query: Option<String>,
}

#[tauri::command]
async fn sessions_page(
    state: tauri::State<'_, AppState>,
    filter: SessionsFilterDto,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<services::session_queries::SessionsPage, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let filter = services::session_queries::SessionFilter {
            agent: filter.agent,
            from: filter.from,
            to: filter.to,
            query: filter.query,
        };
        let limit = limit
            .map(|l| l as usize)
            .unwrap_or(services::session_queries::DEFAULT_SESSION_PAGE_LIMIT);
        services::session_queries::sessions_page(&db, &filter, cursor.as_deref(), limit)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `session_ref` 只含索引身份（源 + 会话 + 可选 actor），不含任何系统路径。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRefDto {
    pub source_id: String,
    pub session_id: String,
    #[serde(default)]
    pub actor_id: Option<String>,
}

/// 正文失败的稳定错误码 + 面向用户的中文说明。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentErrorDto {
    pub code: String,
    pub message: String,
}

impl From<services::session_queries::ContentError> for ContentErrorDto {
    fn from(err: services::session_queries::ContentError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.message(),
        }
    }
}

#[tauri::command]
async fn session_content_page(
    state: tauri::State<'_, AppState>,
    session_ref: SessionRefDto,
    cursor: Option<u64>,
    limit: Option<u32>,
) -> Result<services::session_queries::ContentPage, ContentErrorDto> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| {
            ContentErrorDto::from(services::session_queries::ContentError::Internal(
                e.to_string(),
            ))
        })?;
        let session_ref = services::session_queries::SessionRef {
            source_id: session_ref.source_id,
            session_id: session_ref.session_id,
            actor_id: session_ref.actor_id,
        };
        let limit = limit
            .map(|l| l as usize)
            .unwrap_or(services::session_queries::DEFAULT_CONTENT_PAGE_LIMIT);
        services::session_queries::session_content_page(&db, &session_ref, cursor, limit)
            .map_err(ContentErrorDto::from)
    })
    .await
    .map_err(|e| {
        ContentErrorDto::from(services::session_queries::ContentError::Internal(
            e.to_string(),
        ))
    })?
}

#[tauri::command]
async fn content_view_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        services::session_queries::content_view_enabled(&db).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn content_view_set(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        services::session_queries::content_view_set(&db, enabled).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            std::fs::create_dir_all(&data_dir)?;
            let db = Database::open(&data_dir.join("providock.db"))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let db = Arc::new(Mutex::new(db));
            let vault: Arc<dyn Vault> = Arc::new(vault::KeyringVault::new());
            // 默认开启的后台调度；失败静默降级（查询层记 stale/unavailable）。
            let scheduler = services::account_scheduler::spawn(Arc::clone(&db), Arc::clone(&vault));
            app.manage(AppState {
                db,
                vault,
                scheduler: Mutex::new(Some(scheduler)),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关窗口即取消后台调度（ACCOUNT_ADAPTERS 第 5 节：关窗口后台策略可取消）。
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    if let Ok(mut guard) = state.scheduler.lock() {
                        if let Some(handle) = guard.as_mut() {
                            handle.shutdown();
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            sync_start,
            sources_list,
            usage_summary,
            usage_series,
            accounts_list,
            account_save,
            account_rotate_credential,
            account_refresh,
            account_delete,
            sessions_page,
            session_content_page,
            content_view_status,
            content_view_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

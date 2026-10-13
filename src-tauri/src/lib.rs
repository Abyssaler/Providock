// Tauri 命令壳与业务模块装配。
// domain（契约类型）/ storage（自有 DB）/ ingestion（事务游标与去重）为 T02；
// adapters（Kimi/Codex 纯解析器）+ services.aggregate + IPC 纵线为 T04。

mod account_dto;
pub mod adapters;
pub mod domain;
pub mod gateway;
pub mod ingestion;
#[cfg(target_os = "macos")]
mod menu_bar;
pub mod safepath;
pub mod services;
pub mod storage;
pub mod vault;
#[cfg(target_os = "windows")]
mod windows_tray;

use ingestion::scanner::SourceScanReport;
use serde::{Deserialize, Serialize};
use services::{background::TaskHandle, usage_sync};
use std::sync::{Arc, Mutex};
use storage::Database;
use tauri::Emitter;
use tauri::Manager;
use vault::Vault;

pub struct AppState {
    db: Arc<Mutex<Database>>,
    vault: Arc<dyn Vault>,
    /// 账户查询调度器句柄：Drop/`shutdown` 时取消后台任务。
    scheduler: Mutex<Option<services::account_scheduler::SchedulerHandle>>,
    usage_sync: Arc<usage_sync::UsageSync>,
    usage_scheduler: Mutex<Option<TaskHandle>>,
    #[cfg(target_os = "macos")]
    tray_scheduler: Mutex<Option<TaskHandle>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    identifier: String,
}

/// 最小联通性命令：前端据此区分桌面运行与浏览器预览。
#[tauri::command]
fn app_info(app: tauri::AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        identifier: app.config().identifier.clone(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFilterDto {
    pub from: Option<String>,
    pub to: Option<String>,
    pub agent: Option<String>,
    /// 用量页可见 Agent 过滤；缺省（None）不限制，空列表匹配不到任何行
    pub agents: Option<Vec<String>>,
    pub stream_domain: Option<String>,
    pub source_id: Option<String>,
    pub model: Option<String>,
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
        agents: f.agents,
        stream_domain: f.stream_domain,
        source_id: f.source_id,
        model: f.model,
    }
}

/// 触发一次全源扫描（后台线程；C03：重扫幂等，游标与事件同事务提交）。
#[tauri::command]
async fn sync_start(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Vec<SourceScanReport>, String> {
    let sync = Arc::clone(&state.usage_sync);
    tauri::async_runtime::spawn_blocking(move || {
        sync.run(
            false,
            || false,
            |status| {
                let _ = app.emit(usage_sync::SYNC_EVENT, status);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn usage_sync_status(state: tauri::State<'_, AppState>) -> usage_sync::SyncStatus {
    state.usage_sync.status()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitoringSettings {
    agent_auto_sync_enabled: bool,
    account_auto_refresh_enabled: bool,
    agent_interval_seconds: u64,
    balance_interval_seconds: u64,
    plan_interval_seconds: u64,
}

fn read_monitoring(db: &Database) -> MonitoringSettings {
    let config = services::account_scheduler::load_config(db);
    MonitoringSettings {
        agent_auto_sync_enabled: usage_sync::enabled(db),
        account_auto_refresh_enabled: config.enabled,
        agent_interval_seconds: usage_sync::SYNC_INTERVAL_SECONDS,
        balance_interval_seconds: config.balance_interval_seconds,
        plan_interval_seconds: config.plan_interval_seconds,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitoringPatch {
    agent_auto_sync_enabled: Option<bool>,
    account_auto_refresh_enabled: Option<bool>,
}

#[tauri::command]
async fn monitoring_settings(
    state: tauri::State<'_, AppState>,
) -> Result<MonitoringSettings, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        Ok(read_monitoring(&*db.lock().map_err(|e| e.to_string())?))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn monitoring_update(
    state: tauri::State<'_, AppState>,
    patch: MonitoringPatch,
) -> Result<MonitoringSettings, String> {
    let wake_usage = patch.agent_auto_sync_enabled.is_some();
    let wake_accounts = patch.account_auto_refresh_enabled.is_some();
    let db = Arc::clone(&state.db);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let tx = db.conn().unchecked_transaction().map_err(|e| e.to_string())?;
        for (key, value) in [(usage_sync::AUTO_SYNC_KEY, patch.agent_auto_sync_enabled), (services::account_scheduler::SCHEDULER_ENABLED_KEY, patch.account_auto_refresh_enabled)] {
            if let Some(value) = value {
                tx.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", rusqlite::params![key, if value { "true" } else { "false" }]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok::<_, String>(read_monitoring(&db))
    }).await.map_err(|e| e.to_string())??;
    if wake_usage {
        if let Some(handle) = state
            .usage_scheduler
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
        {
            handle.wake();
        }
    }
    if wake_accounts {
        if let Some(handle) = state.scheduler.lock().map_err(|e| e.to_string())?.as_ref() {
            handle.wake();
        }
    }
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageOptions {
    sources: Vec<UsageSourceOption>,
    models: Vec<String>,
    models_by_agent: std::collections::BTreeMap<String, Vec<String>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageSourceOption {
    source_id: String,
    agent: String,
    label: String,
}

#[tauri::command]
async fn usage_options(state: tauri::State<'_, AppState>) -> Result<UsageOptions, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let mut sources = db.conn().prepare("SELECT source_id, agent, COALESCE(display_label, agent) FROM sources ORDER BY agent, source_id").map_err(|e| e.to_string())?;
        let sources = sources.query_map([], |r| Ok(UsageSourceOption { source_id: r.get(0)?, agent: r.get(1)?, label: r.get(2)? })).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        let mut models = db.conn().prepare("SELECT DISTINCT COALESCE(model_normalized, model_raw) FROM usage_events WHERE stream_domain = 'local_log' AND COALESCE(model_normalized, model_raw) IS NOT NULL ORDER BY 1").map_err(|e| e.to_string())?;
        let models = models.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        let mut pairs=db.conn().prepare("SELECT DISTINCT agent, COALESCE(model_normalized,model_raw) FROM usage_events WHERE stream_domain='local_log' AND COALESCE(model_normalized,model_raw) IS NOT NULL ORDER BY 1,2").map_err(|e|e.to_string())?;
        let rows=pairs.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
        let mut models_by_agent=std::collections::BTreeMap::<String,Vec<String>>::new();
        for row in rows{let(agent,model)=row.map_err(|e|e.to_string())?;models_by_agent.entry(agent).or_default().push(model);}
        Ok(UsageOptions { sources, models, models_by_agent })
    }).await.map_err(|e| e.to_string())?
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
    pub snapshot: Option<account_dto::SnapshotDto>,
    /// 展示用：最近成功快照已过期（其后有失败查询）。
    pub stale: bool,
    /// 最近一行查询的状态（含失败），无任何查询为 null。
    pub latest_status: Option<services::accounts::SnapshotStatus>,
    /// Checks the local credential file only; this never reads Keychain.
    pub credential_ready: bool,
}

fn build_account_summary(
    db: &Database,
    vault: &dyn Vault,
    account_id: &str,
) -> Result<AccountSummaryDto, String> {
    use services::accounts as acc;
    let account = acc::account_get(db, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("account not found: {account_id}"))?;
    let credentials: Vec<CredentialDto> = acc::credentials_list(db, account_id, true)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(CredentialDto::from)
        .collect();
    let (snapshot, stale) = match acc::snapshot_latest(db, account_id).map_err(|e| e.to_string())? {
        Some((snapshot, stale)) => (Some(snapshot.into()), stale),
        None => (None, false),
    };
    let latest_status = acc::snapshot_latest_status(db, account_id).map_err(|e| e.to_string())?;
    let mut credential_ready = matches!(account.platform.as_str(), "codex" | "codex_appserver");
    let mut refs = db
        .conn()
        .prepare("SELECT vault_ref FROM credential_refs WHERE account_id=?1 AND retired_at IS NULL")
        .map_err(|e| e.to_string())?;
    let rows = refs
        .query_map([account_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for reference in rows {
        if vault
            .exists(&reference.map_err(|e| e.to_string())?)
            .unwrap_or(false)
        {
            credential_ready = true;
        }
    }
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
        credential_ready,
    })
}

#[tauri::command]
async fn accounts_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AccountSummaryDto>, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|e| e.to_string())?;
        let accounts = services::accounts::accounts_list(&db, false).map_err(|e| e.to_string())?;
        accounts
            .into_iter()
            .map(|account| build_account_summary(&db, vault.as_ref(), &account.account_id))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
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
    /// 前端仅在提交表单瞬间传入；后端存入用户选择的本地明文凭据文件，响应不回显。
    pub secret: Option<String>,
    pub capabilities: Option<serde_json::Value>,
}

#[tauri::command]
async fn account_save(
    state: tauri::State<'_, AppState>,
    mut input: AccountSaveInput,
) -> Result<AccountSummaryDto, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    let result = tauri::async_runtime::spawn_blocking(move || {
        use services::accounts as acc;
        if input.platform == "codex"
            && (input.secret.is_some()
                || input.credential_type.is_some()
                || input.config_kind != "coding_plan")
        {
            return Err("Codex 官方额度使用本机 ChatGPT 登录，无需上传凭据".into());
        }
        let secret = input.secret.take().map(vault::SecretString::new);
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

        // 只有前端明确提交秘密时才写入本地凭据库；秘密在此作用域内即被清零释放。
        if let Some(secret) = secret {
            let credential_type = input
                .credential_type
                .as_deref()
                .and_then(acc::CredentialType::parse)
                .ok_or_else(|| "credential_type is required when saving a secret".to_owned())?;
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

        build_account_summary(&db, vault.as_ref(), &account_id)
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(handle) = state.scheduler.lock().map_err(|e| e.to_string())?.as_ref() {
        handle.wake();
    }
    Ok(result)
}

#[derive(Deserialize)]
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
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| e.to_string())??;
    if let Some(handle) = state.scheduler.lock().map_err(|e| e.to_string())?.as_ref() {
        handle.wake();
    }
    Ok(result)
}

#[tauri::command]
async fn account_refresh(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<account_dto::OutcomeDto, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    let result = tauri::async_runtime::spawn_blocking(move || {
        services::account_query::query_account_shared(&db, vault.as_ref(), &account_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(handle) = state.scheduler.lock().map_err(|e| e.to_string())?.as_ref() {
        handle.wake();
    }
    Ok(result.into())
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
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| e.to_string())??;
    if let Some(handle) = state.scheduler.lock().map_err(|e| e.to_string())?.as_ref() {
        handle.wake();
    }
    Ok(result)
}

#[tauri::command]
async fn model_prices_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<services::model_prices::ModelPrice>, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        services::model_prices::list(&*db.lock().map_err(|e| e.to_string())?)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn model_price_save(
    state: tauri::State<'_, AppState>,
    input: services::model_prices::ModelPrice,
) -> Result<services::model_prices::ModelPrice, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        services::model_prices::save(&*db.lock().map_err(|e| e.to_string())?, input)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn model_prices_import(
    target: services::price_import::ImportTarget,
) -> Result<services::price_import::PriceQuote, String> {
    if target.model.trim().is_empty() {
        return Err("请先选择 Agent 和模型".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let catalogue = services::price_import::fetch()?;
        services::price_import::quote(&catalogue, target)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn price_parser_accounts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<services::price_page::ParserAccount>, String> {
    services::price_page::parser_accounts(&*state.db.lock().map_err(|e| e.to_string())?)
}
#[tauri::command]
async fn price_parser_models(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        services::price_page::parser_models(&db, vault.as_ref(), &account_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn model_price_from_page(
    state: tauri::State<'_, AppState>,
    input: services::price_page::PageRequest,
) -> Result<services::price_import::PriceQuote, String> {
    let db = Arc::clone(&state.db);
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || {
        services::price_page::extract(&db, vault.as_ref(), input)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn usage_dashboard(
    state: tauri::State<'_, AppState>,
    filter: Option<UsageFilterDto>,
    hourly: bool,
) -> Result<services::usage_details::Dashboard, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        services::usage_details::dashboard(
            &*db.lock().map_err(|e| e.to_string())?,
            &map_filter(filter.unwrap_or_default()),
            hourly,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn usage_events(
    state: tauri::State<'_, AppState>,
    filter: Option<UsageFilterDto>,
    page: u32,
    page_size: u32,
) -> Result<services::usage_details::EventPage, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        services::usage_details::events(
            &*db.lock().map_err(|e| e.to_string())?,
            &map_filter(filter.unwrap_or_default()),
            page,
            page_size,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 用量页 Agent 可见性：settings 单键存储，读取永不失败（缺失/损坏回退全部可见）
// ---------------------------------------------------------------------------

#[tauri::command]
async fn agent_visibility(
    state: tauri::State<'_, AppState>,
) -> Result<services::agent_visibility::AgentVisibilityDto, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        Ok(services::agent_visibility::read(
            &*db.lock().map_err(|e| e.to_string())?,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn agent_visibility_update(
    state: tauri::State<'_, AppState>,
    input: services::agent_visibility::AgentVisibilityUpdate,
) -> Result<services::agent_visibility::AgentVisibilityDto, String> {
    let db = Arc::clone(&state.db);
    tauri::async_runtime::spawn_blocking(move || {
        services::agent_visibility::write(
            &*db.lock().map_err(|e| e.to_string())?,
            &input.visible_agents,
        )
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            std::fs::create_dir_all(&data_dir)?;
            let db = Database::open(&data_dir.join("providock.db"))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let db = Arc::new(Mutex::new(db));
            let vault: Arc<dyn Vault> = Arc::new(vault::LocalPlaintextVault::new(
                data_dir.join("credentials").join("keys.sqlite3"),
            ));
            // 默认开启的后台调度；失败静默降级（查询层记 stale/unavailable）。
            let scheduler = services::account_scheduler::spawn(
                Arc::clone(&db),
                Arc::clone(&vault),
                app.handle().clone(),
            );
            let usage_sync = Arc::new(usage_sync::UsageSync::new(Arc::clone(&db)));
            let usage_scheduler = usage_sync::spawn(Arc::clone(&usage_sync), app.handle().clone());
            app.manage(AppState {
                db,
                vault,
                scheduler: Mutex::new(Some(scheduler)),
                usage_sync,
                usage_scheduler: Mutex::new(Some(usage_scheduler)),
                #[cfg(target_os = "macos")]
                tray_scheduler: Mutex::new(None),
            });
            #[cfg(target_os = "macos")]
            {
                let tray = menu_bar::setup(app.handle())?;
                *app.state::<AppState>().tray_scheduler.lock().unwrap() = Some(tray);
            }
            #[cfg(target_os = "windows")]
            windows_tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            #[cfg(target_os = "windows")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Keep the webview and background tasks alive. If hiding fails, allow
                // the normal close instead of leaving an inaccessible process.
                if window.label() == "main" && window.hide().is_ok() {
                    api.prevent_close();
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            sync_start,
            usage_sync_status,
            monitoring_settings,
            monitoring_update,
            usage_options,
            sources_list,
            usage_summary,
            usage_series,
            accounts_list,
            model_prices_list,
            model_price_save,
            model_prices_import,
            price_parser_accounts,
            price_parser_models,
            model_price_from_page,
            usage_dashboard,
            usage_events,
            agent_visibility,
            agent_visibility_update,
            account_save,
            account_rotate_credential,
            account_refresh,
            account_delete,
            sessions_page,
            session_content_page,
            content_view_status,
            content_view_set,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                menu_bar::show(app);
            }
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut guard) = state.usage_scheduler.lock() {
                        if let Some(handle) = guard.as_mut() {
                            handle.shutdown();
                        }
                    }
                    if let Ok(mut guard) = state.scheduler.lock() {
                        if let Some(handle) = guard.as_mut() {
                            handle.shutdown();
                        }
                    }
                    #[cfg(target_os = "macos")]
                    if let Ok(mut guard) = state.tray_scheduler.lock() {
                        if let Some(handle) = guard.as_mut() {
                            handle.shutdown();
                        }
                    }
                }
            }
        });
}

#[cfg(test)]
mod ipc_regressions {
    use super::*;
    use services::accounts as acc;
    #[test]
    fn usage_filter_agents_maps_through_and_visibility_dto_serializes_camel_case() {
        let dto: UsageFilterDto =
            serde_json::from_value(serde_json::json!({ "agents": ["codex", "kimi_code"] }))
                .unwrap();
        assert_eq!(
            map_filter(dto).agents.as_deref(),
            Some(["codex".to_owned(), "kimi_code".to_owned()].as_slice())
        );
        // 旧前端载荷不含 agents 字段时必须保持 None（行为与之前完全一致）。
        let legacy: UsageFilterDto =
            serde_json::from_value(serde_json::json!({ "agent": "codex" })).unwrap();
        let mapped = map_filter(legacy);
        assert!(mapped.agents.is_none());
        assert_eq!(mapped.agent.as_deref(), Some("codex"));
        let visibility = services::agent_visibility::AgentVisibilityDto {
            visible_agents: vec!["codex".into(), "mcode".into()],
        };
        assert_eq!(
            serde_json::to_value(&visibility).unwrap(),
            serde_json::json!({ "visibleAgents": ["codex", "mcode"] })
        );
    }
    #[test]
    fn stored_account_snapshot_matches_shared_frontend_fixture() {
        let mut db = Database::open_in_memory().unwrap();
        let account = acc::account_create(
            &db,
            &acc::NewAccount::new("deepseek", "合成测试账户", acc::ConfigKind::ApiPayg),
        )
        .unwrap();
        let mut window =
            acc::QuotaWindow::from_used_ratio("synthetic:5h", acc::QuotaUnit::Tokens, "0.58")
                .unwrap();
        window.duration_seconds = Some(18000);
        acc::snapshot_write(
            &mut db,
            &acc::NewSnapshot {
                account_id: account.account_id.clone(),
                queried_at: "2026-10-12T00:00:00Z".into(),
                evidence: Some("synthetic:interop".into()),
                balances: vec![
                    acc::BalanceEntry::new(acc::BalanceKind::Available, "18", "CNY").unwrap(),
                ],
                quota_windows: vec![window],
                usage_metrics: vec![
                    acc::UsageMetric::new("total", "123", acc::QuotaUnit::Tokens).unwrap(),
                ],
                ..Default::default()
            },
        )
        .unwrap();
        let mut actual = serde_json::to_value(
            build_account_summary(&db, &vault::SessionVault::new(), &account.account_id).unwrap(),
        )
        .unwrap();
        actual["accountId"] = "synthetic-account".into();
        actual["createdAt"] = "2026-10-12T00:00:00Z".into();
        actual["snapshot"]["accountId"] = "synthetic-account".into();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/interop/account-summary.json"))
                .unwrap();
        assert_eq!(actual,fixture,"Real DB -> IPC serialization must match the frontend render fixture, including every nested field");
    }
    #[test]
    fn local_key_survives_restart_without_entering_account_ipc_or_main_db() {
        let db = Database::open_in_memory().unwrap();
        let dir =
            std::env::temp_dir().join(format!("providock-ipc-vault-{}", rand::random::<u64>()));
        let path = dir.join("keys.sqlite3");
        let vault = vault::LocalPlaintextVault::new(path.clone());
        let a = acc::account_create(
            &db,
            &acc::NewAccount::new("deepseek", "synthetic", acc::ConfigKind::ApiPayg),
        )
        .unwrap();
        acc::credential_save(
            &db,
            &vault,
            &a.account_id,
            acc::CredentialType::ApiKey,
            None,
            &vault::SecretString::new("synthetic-secret-local-only"),
        )
        .unwrap();
        let first = build_account_summary(&db, &vault, &a.account_id).unwrap();
        assert!(first.credential_ready);
        assert!(!serde_json::to_string(&first)
            .unwrap()
            .contains("synthetic-secret-local-only"));
        let restarted =
            build_account_summary(&db, &vault::LocalPlaintextVault::new(path), &a.account_id)
                .unwrap();
        assert!(restarted.credential_ready);
        assert_eq!(restarted.credentials.len(), 1);
        assert_eq!(restarted.account_id, first.account_id);
        let stored_mask: String = db
            .conn()
            .query_row("SELECT mask FROM credential_refs", [], |r| r.get(0))
            .unwrap();
        assert!(!stored_mask.contains("synthetic-secret-local-only"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

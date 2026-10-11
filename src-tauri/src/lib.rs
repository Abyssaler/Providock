// Tauri 命令壳与业务模块装配。
// domain（契约类型）/ storage（自有 DB）/ ingestion（事务游标与去重）为 T02；
// adapters（Kimi/Codex 纯解析器）+ services.aggregate + IPC 纵线为 T04。

pub mod adapters;
pub mod domain;
pub mod ingestion;
pub mod vault;
pub mod safepath;
pub mod services;
pub mod storage;

use ingestion::scanner::{discover_sources, scan_source, SourceScanReport};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use storage::Database;
use tauri::Manager;

pub struct AppState {
    db: Arc<Mutex<Database>>,
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
            app.manage(AppState {
                db: Arc::new(Mutex::new(db)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            sync_start,
            sources_list,
            usage_summary,
            usage_series,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

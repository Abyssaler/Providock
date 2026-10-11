// 提供 Tauri 命令壳；业务模块（domain/ingestion/storage/vault/...）在 T02 起逐步落地。

use serde::Serialize;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

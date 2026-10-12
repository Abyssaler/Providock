//! macOS native menu-bar lifecycle; independent of a visible webview.
use crate::{
    services::{
        background::{self, TaskHandle},
        menu_usage, usage_sync,
    },
    AppState,
};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager,
};

pub fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
pub fn setup(app: &tauri::AppHandle) -> tauri::Result<TaskHandle> {
    let header = MenuItem::new(app, "最近 24 小时 · 本机日志", false, None::<&str>)?;
    let total = MenuItem::new(app, "已知总量 · 读取中…", false, None::<&str>)?;
    let counts = MenuItem::new(app, "输入 — · 输出 — · 缓存命中 —", false, None::<&str>)?;
    let status = MenuItem::new(app, "采集状态 · 读取中…", false, None::<&str>)?;
    let agents = menu_usage::AGENTS
        .iter()
        .map(|(_, label)| MenuItem::new(app, format!("{label}    —"), false, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let open = MenuItem::with_id(app, "providock-open", "打开 Providock", true, None::<&str>)?;
    let sync = MenuItem::with_id(
        app,
        "providock-sync",
        "立即同步 Agent 用量",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "providock-quit", "退出 Providock", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&header, &total, &counts, &status, &sep1])?;
    for item in &agents {
        menu.append(item)?;
    }
    menu.append_items(&[&sep2, &open, &sync, &sep3, &quit])?;
    TrayIconBuilder::with_id("providock-menu")
        .icon(tauri::image::Image::new_owned(
            include_bytes!("../icons/tray.rgba").to_vec(),
            36,
            36,
        ))
        .icon_as_template(true)
        .tooltip("Providock · 最近 24 小时用量")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "providock-open" => show(app),
            "providock-quit" => app.exit(0),
            "providock-sync" => {
                let sync = app.state::<AppState>().usage_sync.clone();
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = sync.run(
                        false,
                        || false,
                        |status| {
                            let _ = app.emit(usage_sync::SYNC_EVENT, status);
                        },
                    );
                });
            }
            _ => {}
        })
        .build(app)?;
    // Native item updates happen off the UI thread, and never while holding the DB lock.
    let app_handle = app.clone();
    let task = TaskHandle::spawn(move |mut change| async move {
        loop {
            if *change.borrow() {
                return;
            }
            let app = app_handle.clone();
            let total = total.clone();
            let counts = counts.clone();
            let status = status.clone();
            let agents = agents.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<AppState>();
                let result = state
                    .db
                    .lock()
                    .map_err(|_| "数据库暂不可用".to_string())
                    .and_then(|db| {
                        menu_usage::recent(&db, time::OffsetDateTime::now_utc())
                            .map(|usage| (usage, usage_sync::enabled(&db)))
                    });
                match result {
                    Ok((usage, enabled)) => {
                        let f = menu_usage::compact;
                        let _ = total.set_text(format!(
                            "已知小计 {} Token · {} 条计量记录",
                            f(usage.tokens[0]),
                            usage.events
                        ));
                        let _ = counts.set_text(format!(
                            "输入 {} · 输出 {} · 缓存命中 {}",
                            f(usage.tokens[1]),
                            f(usage.tokens[2]),
                            f(usage.tokens[3])
                        ));
                        for ((id, label), item) in menu_usage::AGENTS.iter().zip(&agents) {
                            let _ = item.set_text(format!(
                                "{label}    {} Token",
                                f(usage.agents.get(*id).copied().flatten())
                            ));
                        }
                        let sync = state.usage_sync.status();
                        let info = if sync.running {
                            "正在同步".into()
                        } else if sync.last_error.is_some() {
                            "上次同步失败 · 打开查看".into()
                        } else if let Some(at) = sync.last_completed_at {
                            format!("最近同步 {} UTC", at.get(11..19).unwrap_or("—"))
                        } else {
                            "尚未同步".into()
                        };
                        let _ = status.set_text(format!(
                            "{info} · 自动{}",
                            if enabled { "开启" } else { "关闭" }
                        ));
                    }
                    Err(_) => {
                        let _ = total.set_text("用量暂不可用 · 请打开窗口重试");
                    }
                }
            })
            .await;
            if background::wait(Some(Duration::from_secs(30)), &mut change).await {
                return;
            }
        }
    });
    let handle = app.clone();
    app.listen(usage_sync::SYNC_EVENT, move |_| {
        if let Some(state) = handle.try_state::<AppState>() {
            if let Ok(guard) = state.tray_scheduler.lock() {
                if let Some(task) = guard.as_ref() {
                    task.wake();
                }
            }
        }
    });
    Ok(task)
}

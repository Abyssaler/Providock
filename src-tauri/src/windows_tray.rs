//! Windows close-to-tray lifecycle. Explicit exit uses the shared task shutdown path.
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "providock-open", "打开 Providock", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "providock-quit", "退出 Providock", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
    TrayIconBuilder::with_id("providock-windows-tray")
        // The coloured app icon stays visible on both light and dark Windows taskbars.
        .icon(app.default_window_icon().expect("bundled app icon").clone())
        .tooltip("Providock")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "providock-open" => show(app),
            "providock-quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

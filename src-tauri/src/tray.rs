use crate::config;
use tauri::{menu::MenuBuilder, tray::TrayIconBuilder, AppHandle, Manager};

const TRAY_ID: &str = "nodenet-tray";

#[tauri::command]
pub fn rebuild_tray(app: AppHandle) -> Result<(), String> {
    rebuild_tray_on_main_thread(&app).map_err(|error| error.to_string())
}

pub fn rebuild_tray_on_main_thread(app: &AppHandle) -> tauri::Result<()> {
    let app = app.clone();
    app.clone().run_on_main_thread(move || {
        if let Err(error) = rebuild_tray_for_app(&app) {
            eprintln!("failed to rebuild tray: {error}");
        }
    })
}

pub fn rebuild_tray_for_app(app: &AppHandle) -> tauri::Result<()> {
    let _ = app.remove_tray_by_id(TRAY_ID);
    build_tray(app)
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let mut menu = MenuBuilder::new(app)
        .text("show", "Show NodeNet")
        .separator();

    if let Ok(config) = config::load_config() {
        for server in config.servers {
            menu = menu.text(
                format!("server-{}", server.id),
                format!("* {}", server.name),
            );
        }
    }

    let menu = menu.separator().text("quit", "Quit").build()?;
    let icon = app.default_window_icon().cloned();
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("NodeNet")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == "show" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            } else if id == "quit" {
                app.exit(0);
            } else if id.starts_with("server-") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });

    if let Some(icon) = icon {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

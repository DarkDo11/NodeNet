mod alerts;
mod commands;
mod config;
mod keychain;
mod metrics;
mod secrets;
mod ssh;
mod terminal;
mod three_x_ui;
mod util;

use tauri::{menu::MenuBuilder, tray::TrayIconBuilder, Emitter, Manager, WindowEvent};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::TerminalState::default())
        .manage(alerts::AlertsState::default())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            build_tray(app)?;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = secrets::migrate_plaintext_config_secrets(&app_handle).await {
                    let _ = app_handle.emit(
                        "alert-error",
                        format!("config secret migration failed: {error}"),
                    );
                }
                if let Err(error) = alerts::load_events_into_state(&app_handle).await {
                    let _ = app_handle.emit("alert-error", format!("events load failed: {error}"));
                }
                alerts::start_alert_poller(app_handle);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config_path,
            commands::get_app_config,
            commands::save_app_config,
            commands::get_servers,
            commands::upsert_server,
            commands::delete_server,
            commands::set_poll_interval,
            commands::set_theme,
            commands::get_metrics,
            commands::ping_server,
            commands::save_ssh_password,
            commands::delete_ssh_password,
            commands::save_ssh_key_passphrase,
            commands::delete_ssh_key_passphrase,
            commands::save_three_x_ui_password,
            commands::delete_three_x_ui_password,
            commands::get_inbounds,
            commands::get_clients,
            commands::add_client,
            commands::delete_client,
            commands::reset_client_traffic,
            commands::extend_client,
            commands::generate_client_link,
            commands::restart_xray,
            commands::reboot_server,
            commands::download_config,
            commands::reset_all_expired_clients,
            commands::delete_all_disabled_clients,
            commands::export_clients_csv,
            commands::test_server_connection,
            commands::load_metrics_cache,
            commands::save_metrics_cache,
            terminal::terminal_connect,
            terminal::terminal_input,
            terminal::terminal_resize,
            terminal::terminal_disconnect,
            alerts::get_events
        ])
        .run(tauri::generate_context!())
        .expect("error while running NodeNet");
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
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
    let mut tray = TrayIconBuilder::new()
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

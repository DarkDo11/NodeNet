mod alerts;
mod commands;
mod config;
mod keychain;
mod metrics;
mod secrets;
mod ssh;
mod terminal;
mod three_x_ui;

use tauri::Emitter;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::TerminalState::default())
        .manage(alerts::AlertsState::default())
        .setup(|app| {
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
            terminal::terminal_connect,
            terminal::terminal_input,
            terminal::terminal_resize,
            terminal::terminal_disconnect,
            alerts::get_events
        ])
        .run(tauri::generate_context!())
        .expect("error while running NodeNet");
}

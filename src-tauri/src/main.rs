mod alerts;
mod commands;
mod config;
mod keychain;
mod metrics;
mod monitor;
mod secrets;
mod ssh;
mod terminal;
mod three_x_ui;
mod tray;
mod util;

use tauri::{Emitter, Listener, WindowEvent};

fn main() {
    ssh::cleanup_stale_sockets();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(terminal::TerminalState::default())
        .manage(alerts::AlertsState::default())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            tray::build_tray(app.handle())?;
            let tray_app_handle = app.handle().clone();
            app.listen("servers-changed", move |_| {
                let _ = tray::rebuild_tray_on_main_thread(&tray_app_handle);
            });
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
            commands::upsert_bastion,
            commands::delete_bastion,
            commands::set_poll_interval,
            commands::set_theme,
            commands::set_monitor_server,
            commands::set_monitor_target,
            commands::install_monitor_agent,
            commands::reinstall_monitor_agent,
            commands::sync_monitor_ssh_key,
            commands::list_monitor_servers,
            commands::delete_monitor_server,
            commands::get_metrics,
            commands::ping_server,
            commands::save_ssh_password,
            commands::delete_ssh_password,
            commands::save_bastion_password,
            commands::delete_bastion_password,
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
            commands::run_preset_command,
            commands::run_streaming_command,
            commands::get_remote_logs,
            commands::get_xray_config,
            commands::save_xray_config,
            commands::upload_routing_file,
            commands::get_panel_setup_info_command,
            commands::list_ssh_public_keys,
            commands::read_ssh_public_key,
            commands::create_ssh_key_pair,
            commands::load_metrics_cache,
            commands::save_metrics_cache,
            tray::rebuild_tray,
            terminal::terminal_connect,
            terminal::terminal_input,
            terminal::terminal_resize,
            terminal::terminal_disconnect,
            alerts::get_events
        ])
        .run(tauri::generate_context!())
        .expect("error while running NodeNet");
}

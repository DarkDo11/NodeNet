mod commands;
mod config;
mod metrics;
mod ssh;
mod three_x_ui;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_config_path,
            commands::get_servers,
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
            commands::download_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running vpnctrl");
}

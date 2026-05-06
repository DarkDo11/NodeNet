use crate::{
    config::{
        config_path, delete_server as delete_server_config, find_server, load_config, save_config,
        set_poll_interval as set_poll_interval_config, set_theme as set_theme_config,
        upsert_server as upsert_server_config, AppConfig, ServerConfig,
    },
    metrics::{collect, ServerMetrics},
    ssh::{delete_password, ping, save_password, PingResult},
    three_x_ui::{self, ThreeXClient, ThreeXInbound},
};
use tauri::AppHandle;

#[tauri::command]
pub fn get_config_path() -> Result<String, String> {
    config_path()
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_servers() -> Result<Vec<ServerConfig>, String> {
    load_config()
        .map(|config| config.servers)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_app_config() -> Result<AppConfig, String> {
    load_config().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_app_config(config: AppConfig) -> Result<AppConfig, String> {
    save_config(&config).map_err(|error| error.to_string())?;
    Ok(config)
}

#[tauri::command]
pub fn upsert_server(server: ServerConfig) -> Result<AppConfig, String> {
    upsert_server_config(server).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_server(server_id: String) -> Result<AppConfig, String> {
    delete_server_config(&server_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_poll_interval(seconds: u64) -> Result<AppConfig, String> {
    set_poll_interval_config(seconds).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_theme(theme: String) -> Result<AppConfig, String> {
    set_theme_config(theme).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_metrics(app: AppHandle, server_id: String) -> Result<ServerMetrics, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    collect(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ping_server(server_id: String) -> Result<PingResult, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    Ok(ping(&server).await)
}

#[tauri::command]
pub async fn save_ssh_password(
    app: AppHandle,
    server_id: String,
    password: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    save_password(&app, &server, &password)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_ssh_password(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    delete_password(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_three_x_ui_password(
    app: AppHandle,
    server_id: String,
    username: String,
    password: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::save_credentials(&app, &server, &username, &password)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_three_x_ui_password(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::delete_credentials(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_inbounds(app: AppHandle, server_id: String) -> Result<Vec<ThreeXInbound>, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::get_inbounds(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_clients(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
) -> Result<Vec<ThreeXClient>, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::get_clients(&app, &server, inbound_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn add_client(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
    name: String,
    limit_gb: f64,
    expire_days: i64,
) -> Result<ThreeXClient, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::add_client(&app, &server, inbound_id, name, limit_gb, expire_days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_client(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::delete_client(&app, &server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reset_client_traffic(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::reset_client_traffic(&app, &server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn extend_client(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
    client_id: String,
    days: i64,
) -> Result<ThreeXClient, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::extend_client(&app, &server, inbound_id, client_id, days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn generate_client_link(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::generate_link(&app, &server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn restart_xray(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::restart_xray(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reboot_server(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::reboot_server(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn download_config(app: AppHandle, server_id: String) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::download_config(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

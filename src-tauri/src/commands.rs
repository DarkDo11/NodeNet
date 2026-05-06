use crate::{
    config::{
        config_path, delete_server as delete_server_config, find_server, load_config, save_config,
        set_poll_interval as set_poll_interval_config, set_theme as set_theme_config,
        upsert_server as upsert_server_config, AppConfig, ServerConfig,
    },
    metrics::{collect, ServerMetrics},
    ssh::{
        delete_key_passphrase, delete_password, ping, save_key_passphrase, save_password,
        PingResult,
    },
    three_x_ui::{self, ThreeXClient, ThreeXInbound},
};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use tauri::AppHandle;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub ping: PingResult,
    pub ssh_ok: bool,
    pub ssh_message: String,
    pub panel_ok: Option<bool>,
    pub panel_message: Option<String>,
}

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
pub async fn save_ssh_key_passphrase(
    app: AppHandle,
    server_id: String,
    passphrase: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    save_key_passphrase(&app, &server, &passphrase)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_ssh_key_passphrase(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    delete_key_passphrase(&app, &server)
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

#[tauri::command]
pub async fn reset_all_expired_clients(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
) -> Result<usize, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::reset_all_expired_clients(&app, &server, inbound_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_all_disabled_clients(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
) -> Result<usize, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::delete_all_disabled_clients(&app, &server, inbound_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn export_clients_csv(
    app: AppHandle,
    server_id: String,
    inbound_id: i64,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::export_clients_csv(&app, &server, inbound_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn test_server_connection(
    app: AppHandle,
    server: ServerConfig,
    ssh_password: Option<String>,
    ssh_key_passphrase: Option<String>,
    panel_password: Option<String>,
) -> Result<TestConnectionResult, String> {
    if let Some(password) = ssh_password.filter(|value| !value.is_empty()) {
        save_password(&app, &server, &password)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Some(passphrase) = ssh_key_passphrase.filter(|value| !value.is_empty()) {
        save_key_passphrase(&app, &server, &passphrase)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Some(password) = panel_password.filter(|value| !value.is_empty()) {
        three_x_ui::save_credentials(
            &app,
            &server,
            server.panel_user.as_deref().unwrap_or("admin"),
            &password,
        )
        .await
        .map_err(|error| error.to_string())?;
    }

    let ping_result = ping(&server).await;
    let (ssh_ok, ssh_message) = match collect(&app, &server).await {
        Ok(_) => (true, "SSH OK".to_string()),
        Err(error) => (false, error.to_string()),
    };
    let (panel_ok, panel_message) = if server.panel_url.is_some() {
        match three_x_ui::get_inbounds(&app, &server).await {
            Ok(_) => (Some(true), Some("Panel OK".to_string())),
            Err(error) => (Some(false), Some(error.to_string())),
        }
    } else {
        (None, None)
    };

    Ok(TestConnectionResult {
        ping: ping_result,
        ssh_ok,
        ssh_message,
        panel_ok,
        panel_message,
    })
}

#[tauri::command]
pub fn load_metrics_cache() -> Result<Value, String> {
    let path = crate::config::config_dir()
        .map_err(|error| error.to_string())?
        .join("metrics-cache.json");
    if !path.exists() {
        return Ok(Value::Object(Default::default()));
    }
    let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_metrics_cache(cache: Value) -> Result<(), String> {
    let directory = crate::config::config_dir().map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join("metrics-cache.json");
    let raw = serde_json::to_string_pretty(&cache).map_err(|error| error.to_string())?;
    fs::write(path, raw).map_err(|error| error.to_string())
}

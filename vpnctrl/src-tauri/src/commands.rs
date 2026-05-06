use crate::{
    config::{config_path, find_server, load_config, ServerConfig},
    metrics::{collect, ServerMetrics},
    ssh::{delete_password, ping, save_password, PingResult},
    three_x_ui::{self, ThreeXClient, ThreeXInbound},
};

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
pub async fn get_metrics(server_id: String) -> Result<ServerMetrics, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    collect(&server).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ping_server(server_id: String) -> Result<PingResult, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    Ok(ping(&server).await)
}

#[tauri::command]
pub async fn save_ssh_password(server_id: String, password: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    save_password(&server, &password)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_ssh_password(server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    delete_password(&server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_three_x_ui_password(
    server_id: String,
    username: String,
    password: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::save_credentials(&server, &username, &password)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_three_x_ui_password(server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::delete_credentials(&server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_inbounds(server_id: String) -> Result<Vec<ThreeXInbound>, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::get_inbounds(&server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_clients(server_id: String, inbound_id: i64) -> Result<Vec<ThreeXClient>, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::get_clients(&server, inbound_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn add_client(
    server_id: String,
    inbound_id: i64,
    name: String,
    limit_gb: f64,
    expire_days: i64,
) -> Result<ThreeXClient, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::add_client(&server, inbound_id, name, limit_gb, expire_days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_client(
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::delete_client(&server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reset_client_traffic(
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::reset_client_traffic(&server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn extend_client(
    server_id: String,
    inbound_id: i64,
    client_id: String,
    days: i64,
) -> Result<ThreeXClient, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::extend_client(&server, inbound_id, client_id, days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn generate_client_link(
    server_id: String,
    inbound_id: i64,
    client_id: String,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::generate_link(&server, inbound_id, client_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn restart_xray(server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::restart_xray(&server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reboot_server(server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::reboot_server(&server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn download_config(server_id: String) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::download_config(&server)
        .await
        .map_err(|error| error.to_string())
}

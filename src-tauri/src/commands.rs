use crate::{
    alerts,
    config::{
        config_path, delete_bastion as delete_bastion_config,
        delete_server as delete_server_config, find_server, load_config, save_config,
        set_monitor_server as set_monitor_server_config,
        set_monitor_target as set_monitor_target_config,
        set_poll_interval as set_poll_interval_config, set_theme as set_theme_config,
        upsert_bastion as upsert_bastion_config, upsert_server as upsert_server_config, AppConfig,
        BastionConfig, ServerConfig,
    },
    metrics::{collect, ServerMetrics},
    monitor,
    ssh::{
        self, delete_bastion_password as delete_bastion_password_secret, delete_key_passphrase,
        delete_password, ping, read_saved_key_passphrase,
        save_bastion_password as save_bastion_password_secret, save_key_passphrase, save_password,
        PingResult,
    },
    three_x_ui::{self, ThreeXClient, ThreeXInbound},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub ping: PingResult,
    pub ssh_ok: bool,
    pub ssh_message: String,
    pub panel_ok: Option<bool>,
    pub panel_message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelSetupInfo {
    pub port: u16,
    pub username: String,
    pub password: String,
    pub web_base_path: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyPair {
    pub private_key_path: String,
    pub public_key_path: String,
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
pub fn upsert_server(app: AppHandle, server: ServerConfig) -> Result<AppConfig, String> {
    let config = upsert_server_config(server).map_err(|error| error.to_string())?;
    let _ = app.emit("servers-changed", ());
    Ok(config)
}

#[tauri::command]
pub fn upsert_bastion(app: AppHandle, bastion: BastionConfig) -> Result<AppConfig, String> {
    let config = upsert_bastion_config(bastion).map_err(|error| error.to_string())?;
    let _ = app.emit("servers-changed", ());
    Ok(config)
}

#[tauri::command]
pub fn delete_bastion(app: AppHandle, bastion_id: String) -> Result<AppConfig, String> {
    let config = delete_bastion_config(&bastion_id).map_err(|error| error.to_string())?;
    let _ = app.emit("servers-changed", ());
    Ok(config)
}

#[tauri::command]
pub async fn delete_server(app: AppHandle, server_id: String) -> Result<AppConfig, String> {
    let server = load_config()
        .map_err(|error| error.to_string())?
        .servers
        .into_iter()
        .find(|server| server.id == server_id);

    if let Some(server) = &server {
        ssh::close_server_connections(server).await;
        three_x_ui::clear_server_cache(server).await;
        delete_password(&app, server)
            .await
            .map_err(|error| error.to_string())?;
        delete_key_passphrase(&app, server)
            .await
            .map_err(|error| error.to_string())?;
        if server
            .bastion_host
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            delete_bastion_password_secret(&app, server)
                .await
                .map_err(|error| error.to_string())?;
        }
        three_x_ui::delete_credentials(&app, server)
            .await
            .map_err(|error| error.to_string())?;
        remove_server_from_metrics_cache(&server.id).map_err(|error| error.to_string())?;
        alerts::remove_events_for_server(&app, &server.id)
            .await
            .map_err(|error| error.to_string())?;
    }

    let config = delete_server_config(&server_id).map_err(|error| error.to_string())?;
    let _ = app.emit("servers-changed", ());
    Ok(config)
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
pub fn set_monitor_server(server_id: Option<String>) -> Result<AppConfig, String> {
    set_monitor_server_config(server_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_monitor_target(
    server_id: Option<String>,
    bastion_id: Option<String>,
) -> Result<AppConfig, String> {
    set_monitor_target_config(server_id, bastion_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn install_monitor_agent(app: AppHandle) -> Result<String, String> {
    monitor::install_agent(&app)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reinstall_monitor_agent(app: AppHandle) -> Result<String, String> {
    monitor::reinstall_agent(&app)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn sync_monitor_ssh_key(app: AppHandle, server_id: String) -> Result<String, String> {
    monitor::sync_server_ssh_key(&app, &server_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_monitor_servers(
    app: AppHandle,
) -> Result<Vec<monitor::MonitorSavedServer>, String> {
    monitor::list_saved_servers(&app)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_monitor_server(app: AppHandle, server_id: String) -> Result<String, String> {
    monitor::delete_saved_server(&app, &server_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_metrics(app: AppHandle, server_id: String) -> Result<ServerMetrics, String> {
    if let Ok(Ok(Some(metrics))) = tokio::time::timeout(
        Duration::from_secs(6),
        monitor::latest_metrics(&app, &server_id),
    )
    .await
    {
        return Ok(metrics);
    }

    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    match tokio::time::timeout(Duration::from_secs(18), collect(&app, &server)).await {
        Ok(Ok(metrics)) => Ok(metrics),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("metrics collection timed out".to_string()),
    }
}

#[tauri::command]
pub async fn ping_server(app: AppHandle, server_id: String) -> Result<PingResult, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    if let Some(result) = monitor::ping_from_monitor(&app, &server)
        .await
        .map_err(|error| error.to_string())?
    {
        if result.status != "unknown" {
            return Ok(result);
        }
    }

    Ok(ping(&app, &server).await)
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
pub async fn save_bastion_password(
    app: AppHandle,
    server_id: String,
    password: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    save_bastion_password_secret(&app, &server, &password)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_bastion_password(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    delete_bastion_password_secret(&app, &server)
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
    bastion_password: Option<String>,
    panel_password: Option<String>,
) -> Result<TestConnectionResult, String> {
    let ssh_password = ssh_password.filter(|value| !value.is_empty());
    let ssh_key_passphrase = ssh_key_passphrase.filter(|value| !value.is_empty());
    let bastion_password = bastion_password.filter(|value| !value.is_empty());
    let panel_password = panel_password.filter(|value| !value.is_empty());

    let previous_ssh_password = if ssh_password.is_some() {
        ssh::read_password(&app, &server)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let previous_key_passphrase = if ssh_key_passphrase.is_some() {
        read_saved_key_passphrase(&app, &server)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let previous_bastion_password = if bastion_password.is_some() {
        ssh::read_bastion_password(&app, &server)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let previous_panel_credentials = if panel_password.is_some() {
        three_x_ui::read_saved_credentials(&app, &server)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };

    let result = async {
        if let Some(password) = &ssh_password {
            save_password(&app, &server, password)
                .await
                .map_err(|error| error.to_string())?;
        }
        if let Some(passphrase) = &ssh_key_passphrase {
            save_key_passphrase(&app, &server, passphrase)
                .await
                .map_err(|error| error.to_string())?;
        }
        if let Some(password) = &bastion_password {
            save_bastion_password_secret(&app, &server, password)
                .await
                .map_err(|error| error.to_string())?;
        }
        if let Some(password) = &panel_password {
            three_x_ui::save_credentials(
                &app,
                &server,
                server.panel_user.as_deref().unwrap_or("admin"),
                password,
            )
            .await
            .map_err(|error| error.to_string())?;
        }

        let ping_result = ping(&app, &server).await;
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
    .await;

    if ssh_password.is_some() || ssh_key_passphrase.is_some() || bastion_password.is_some() {
        ssh::close_server_connections(&server).await;
    }
    if panel_password.is_some() || bastion_password.is_some() {
        three_x_ui::clear_server_cache(&server).await;
    }

    let mut restore_errors: Vec<String> = Vec::new();

    if ssh_password.is_some() {
        if let Err(error) = match previous_ssh_password {
            Some(password) => save_password(&app, &server, &password).await,
            None => delete_password(&app, &server).await,
        } {
            restore_errors.push(format!("SSH password: {error}"));
        }
    }
    if ssh_key_passphrase.is_some() {
        if let Err(error) = match previous_key_passphrase {
            Some(passphrase) => save_key_passphrase(&app, &server, &passphrase).await,
            None => delete_key_passphrase(&app, &server).await,
        } {
            restore_errors.push(format!("SSH key passphrase: {error}"));
        }
    }
    if bastion_password.is_some() {
        if let Err(error) = match previous_bastion_password {
            Some(password) => save_bastion_password_secret(&app, &server, &password).await,
            None => delete_bastion_password_secret(&app, &server).await,
        } {
            restore_errors.push(format!("bastion password: {error}"));
        }
    }
    if panel_password.is_some() {
        if let Err(error) = match previous_panel_credentials {
            Some((username, password)) => {
                three_x_ui::save_credentials(&app, &server, &username, &password).await
            }
            None => three_x_ui::delete_credentials(&app, &server).await,
        } {
            restore_errors.push(format!("panel credentials: {error}"));
        }
    }

    if !restore_errors.is_empty() {
        let _ = app.emit(
            "alert-error",
            format!(
                "credential restore failed after connection test: {}",
                restore_errors.join("; ")
            ),
        );
    }

    result
}

#[tauri::command]
pub async fn run_preset_command(
    app: AppHandle,
    server_id: String,
    command: String,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    ssh::execute_combined(&app, &server, &command, 900)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_streaming_command(
    app: AppHandle,
    server_id: String,
    command: String,
    session_id: String,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    ssh::execute_streaming_combined(&app, &server, &command, &session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_remote_logs(
    app: AppHandle,
    target_kind: String,
    target_id: Option<String>,
    log_kind: String,
) -> Result<String, String> {
    let target = logs_target_server(&target_kind, target_id.as_deref())
        .map_err(|error| error.to_string())?;
    let command = logs_command(&log_kind);
    ssh::execute_combined(&app, &target, command, 60)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_xray_config(app: AppHandle, server_id: String) -> Result<Value, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::get_xray_config(&app, &server)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_xray_config(
    app: AppHandle,
    server_id: String,
    config: Value,
) -> Result<(), String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::save_xray_config(&app, &server, config)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn upload_routing_file(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_filename: Option<String>,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    three_x_ui::upload_routing_file(&app, &server, &local_path, remote_filename)
        .await
        .map_err(|error| error.to_string())
}

fn logs_target_server(target_kind: &str, target_id: Option<&str>) -> anyhow::Result<ServerConfig> {
    let config = load_config()?;
    match target_kind {
        "monitor" => monitor::monitor_server(&config)?
            .ok_or_else(|| anyhow::anyhow!("monitor target is not configured")),
        "server" => {
            let server_id = target_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("server is not selected"))?;
            config
                .servers
                .into_iter()
                .find(|server| server.id == server_id)
                .ok_or_else(|| anyhow::anyhow!("server '{server_id}' was not found"))
        }
        "bastion" => {
            let bastion_id = target_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("bastion is not selected"))?;
            let bastion = config
                .bastions
                .into_iter()
                .find(|bastion| bastion.id == bastion_id)
                .ok_or_else(|| anyhow::anyhow!("bastion '{bastion_id}' was not found"))?;
            Ok(server_from_bastion_for_logs(&bastion))
        }
        other => Err(anyhow::anyhow!("unknown logs target '{other}'")),
    }
}

fn logs_command(log_kind: &str) -> &'static str {
    match log_kind {
        "monitor" => {
            "journalctl -u nodenet-monitor.service -u nodenet-monitor.timer -n 240 --no-pager -o short-iso 2>&1 || systemctl status nodenet-monitor.service nodenet-monitor.timer --no-pager 2>&1 || true"
        }
        "panel" => {
            "journalctl -u x-ui -u 3x-ui -u xray -n 240 --no-pager -o short-iso 2>&1 || tail -n 240 /var/log/x-ui.log /var/log/xray/*.log 2>&1 || true"
        }
        _ => {
            "journalctl -n 240 --no-pager -o short-iso 2>&1 || tail -n 240 /var/log/syslog /var/log/messages 2>&1 || true"
        }
    }
}

fn server_from_bastion_for_logs(bastion: &BastionConfig) -> ServerConfig {
    ServerConfig {
        id: format!("bastion:{}", bastion.id),
        name: bastion.name.clone(),
        host: bastion.host.clone(),
        ssh_port: bastion.port,
        ssh_user: bastion.user.clone(),
        country: "US".to_string(),
        panel_url: None,
        panel_user: None,
        ssh_key_path: bastion.ssh_key_path.clone(),
        bastion_host: None,
        bastion_port: None,
        bastion_user: None,
        bastion_ssh_key_path: None,
        ssh_key_passphrase: None,
        ssl_verify: false,
    }
}

#[tauri::command(rename = "get_panel_setup_info")]
pub async fn get_panel_setup_info_command(
    app: AppHandle,
    server_id: String,
) -> Result<PanelSetupInfo, String> {
    let mut server = find_server(&server_id).map_err(|error| error.to_string())?;
    let info = get_panel_setup_info(&app, &server)
        .await
        .map_err(|error| error.to_string())?;

    let mut config = load_config().map_err(|error| error.to_string())?;
    let mut config_changed = false;
    if let Some(existing) = config
        .servers
        .iter_mut()
        .find(|existing| existing.id == server_id)
    {
        existing.panel_url = Some(panel_url_from_setup_info(
            &existing.host,
            info.port,
            &info.web_base_path,
        ));
        existing.panel_user = Some(info.username.clone());
        server = existing.clone();
        config_changed = true;
    }
    if config_changed {
        save_config(&config).map_err(|error| error.to_string())?;
    }
    if !info.password.is_empty() {
        three_x_ui::save_credentials(&app, &server, &info.username, &info.password)
            .await
            .map_err(|error| error.to_string())?;
    } else {
        // Plaintext password not found (newer 3x-ui stores only bcrypt in users table).
        // Clear any stale/hashed credentials so the user gets a clean prompt to enter manually.
        let _ = three_x_ui::delete_credentials(&app, &server).await;
        three_x_ui::clear_server_cache(&server).await;
    }
    let _ = app.emit("servers-changed", ());

    Ok(info)
}

pub async fn get_panel_setup_info(
    app: &AppHandle,
    server: &ServerConfig,
) -> anyhow::Result<PanelSetupInfo> {
    ensure_xui_available(app, server).await?;

    let output = ssh::execute_combined(app, server, "x-ui settings 2>&1 || true", 60).await?;
    let mut info = parse_panel_setup_info(&output, "cli");

    let sqlite_username = read_xui_sqlite_setting(app, server, "webUsername")
        .await?
        .or(read_xui_sqlite_user(app, server).await?);
    let sqlite_password = read_xui_sqlite_setting(app, server, "webPassword")
        .await?
        .or(read_xui_sqlite_user_password(app, server).await?);
    let sqlite_port = read_xui_sqlite_setting(app, server, "webPort").await?;
    let sqlite_web_base_path = read_xui_sqlite_setting(app, server, "webBasePath").await?;
    let mut sqlite_found = false;
    if let Some(username) = sqlite_username {
        info.username = username;
        sqlite_found = true;
    }
    if let Some(port) = sqlite_port.and_then(|value| value.parse::<u16>().ok()) {
        info.port = port;
        sqlite_found = true;
    }
    if let Some(web_base_path) = sqlite_web_base_path {
        info.web_base_path = normalize_panel_base_path(&web_base_path);
        sqlite_found = true;
    }
    if let Some(password) = sqlite_password {
        info.password = password;
        info.source = "sqlite".to_string();
        return Ok(info);
    }
    if !info.password.is_empty() {
        return Ok(info);
    }
    if sqlite_found {
        info.source = "sqlite".to_string();
    }

    let fallback_output = ssh::execute_combined(
        app,
        server,
        "x-ui | grep -E \"(port|user|pass)\" 2>/dev/null || true",
        60,
    )
    .await?;
    let fallback_info = parse_panel_setup_info(&fallback_output, "fallback");
    merge_panel_setup_info(&mut info, fallback_info);
    if !info.password.is_empty() {
        info.source = "fallback".to_string();
        return Ok(info);
    }

    if info.source == "cli" {
        info.source = "default".to_string();
    }
    Ok(info)
}

async fn ensure_xui_available(app: &AppHandle, server: &ServerConfig) -> anyhow::Result<()> {
    let command = [
        "command -v x-ui >/dev/null 2>&1",
        "test -f /etc/x-ui/x-ui.db",
        "test -f /usr/local/x-ui/config.json",
        "systemctl list-unit-files 'x-ui.service' '3x-ui.service' --no-legend 2>/dev/null | grep -q .",
    ]
    .join(" || ");
    let output = ssh::execute_combined(
        app,
        server,
        &format!("{command}; printf 'xui_available=%s\\n' \"$?\""),
        60,
    )
    .await?;
    let available = output
        .lines()
        .find_map(|line| line.strip_prefix("xui_available="))
        .is_some_and(|value| value.trim() == "0");

    if available {
        Ok(())
    } else {
        anyhow::bail!("3x-ui is not installed or is not visible on this server")
    }
}

#[tauri::command]
pub fn list_ssh_public_keys() -> Result<Vec<String>, String> {
    let home = directories::BaseDirs::new()
        .ok_or_else(|| "unable to resolve user directories".to_string())?
        .home_dir()
        .join(".ssh");
    if !home.exists() {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();
    for entry in fs::read_dir(home).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "pub")
        {
            keys.push(path.display().to_string());
        }
    }
    keys.sort();
    Ok(keys)
}

#[tauri::command]
pub fn read_ssh_public_key(path: String) -> Result<String, String> {
    let path = expand_public_key_path(&path)?;
    fs::read_to_string(path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_ssh_key_pair(server_id: String, key_name: String) -> Result<SshKeyPair, String> {
    let ssh_dir = directories::BaseDirs::new()
        .ok_or_else(|| "unable to resolve user directories".to_string())?
        .home_dir()
        .join(".ssh");
    fs::create_dir_all(&ssh_dir).map_err(|error| error.to_string())?;

    let key_name = sanitize_ssh_key_name(&key_name)?;
    let mut private_key_path = ssh_dir.join(&key_name);
    let mut index = 2;
    while private_key_path.exists() || public_key_path_for_private(&private_key_path).exists() {
        private_key_path = ssh_dir.join(format!("{key_name}_{index}"));
        index += 1;
    }
    let public_key_path = public_key_path_for_private(&private_key_path);

    let output = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            &private_key_path.display().to_string(),
            "-N",
            "",
            "-C",
            &format!("nodenet-{server_id}"),
        ])
        .output()
        .map_err(|error| format!("failed to run ssh-keygen: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh-keygen failed: {}", stderr.trim()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&private_key_path)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&private_key_path, permissions).map_err(|error| error.to_string())?;
    }

    Ok(SshKeyPair {
        private_key_path: private_key_path.display().to_string(),
        public_key_path: public_key_path.display().to_string(),
    })
}

fn sanitize_ssh_key_name(key_name: &str) -> Result<String, String> {
    let sanitized = key_name
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '_', '-'])
        .to_string();

    if sanitized.is_empty() {
        Err("SSH key name is required".to_string())
    } else {
        Ok(sanitized)
    }
}

fn public_key_path_for_private(private_key_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.pub", private_key_path.display()))
}

#[tauri::command]
pub async fn load_metrics_cache(app: AppHandle) -> Result<Value, String> {
    // Always read the local cache first — it is the persisted baseline from
    // the previous session (written by save_metrics_cache).
    let path = crate::config::config_dir()
        .map_err(|e| e.to_string())?
        .join("metrics-cache.json");
    let local_cache: Value = if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };

    let monitor_enabled = crate::config::load_config()
        .map(|cfg| monitor::is_enabled(&cfg))
        .unwrap_or(false);

    if !monitor_enabled {
        return Ok(local_cache);
    }

    let has_local_data = local_cache.as_object().is_some_and(|o| !o.is_empty());

    if has_local_data {
        // Delta load: only download points newer than what we already have.
        let since = latest_timestamps(&local_cache);
        if let Ok(delta) = monitor::fetch_metrics_delta(&app, &since).await {
            let mut merged = local_cache;
            merge_cache_delta(&mut merged, delta);
            return Ok(merged);
        }
        // Monitor unreachable — return the local cache so the UI isn't blank.
        return Ok(local_cache);
    }

    // No local cache yet: download the full history from the monitor.
    if let Ok(Some(full)) = monitor::load_metrics_cache(&app).await {
        return Ok(full);
    }
    Ok(Value::Object(Default::default()))
}

#[tauri::command]
pub fn save_metrics_cache(cache: Value) -> Result<(), String> {
    // Always persist locally — even in monitor mode — so the next startup can
    // do incremental delta loading instead of downloading the full history.
    let directory = crate::config::config_dir().map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join("metrics-cache.json");
    let raw = serde_json::to_string_pretty(&cache).map_err(|error| error.to_string())?;
    fs::write(path, raw).map_err(|error| error.to_string())
}

fn remove_server_from_metrics_cache(server_id: &str) -> anyhow::Result<()> {
    let directory = crate::config::config_dir()?;
    let path = directory.join("metrics-cache.json");
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&path)?;
    let mut cache = serde_json::from_str::<Value>(&raw)?;
    if let Some(object) = cache.as_object_mut() {
        object.remove(server_id);
    }
    let raw = serde_json::to_string_pretty(&cache)?;
    fs::write(path, raw)?;
    Ok(())
}

fn parse_panel_setup_info(output: &str, source: &str) -> PanelSetupInfo {
    let mut port = None;
    let mut username = None;
    let mut password = None;
    let mut web_base_path = None;

    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("port") {
            port = port.or_else(|| first_u16(line));
        }
        if lower.contains("username") || lower.contains("user") {
            username = username.or_else(|| value_after_separator(line));
        }
        if lower.contains("password") || lower.contains("pass") {
            password = password.or_else(|| value_after_separator(line));
        }
        if lower.contains("webbasepath") || lower.contains("web base path") {
            web_base_path = web_base_path.or_else(|| value_after_separator(line));
        }
    }

    PanelSetupInfo {
        port: port.unwrap_or(65333),
        username: username.unwrap_or_else(|| "admin".to_string()),
        password: password.unwrap_or_default(),
        web_base_path: web_base_path
            .map(|value| normalize_panel_base_path(&value))
            .unwrap_or_default(),
        source: source.to_string(),
    }
}

async fn read_xui_sqlite_setting(
    app: &AppHandle,
    server: &ServerConfig,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let command = format!(
        "sqlite3 /etc/x-ui/x-ui.db \"SELECT value FROM settings WHERE key='{}' LIMIT 1;\" 2>/dev/null || true",
        key.replace('\'', "''")
    );
    read_xui_sqlite_value(app, server, &command).await
}

async fn read_xui_sqlite_user(
    app: &AppHandle,
    server: &ServerConfig,
) -> anyhow::Result<Option<String>> {
    read_xui_sqlite_value(
        app,
        server,
        "sqlite3 /etc/x-ui/x-ui.db \"SELECT username FROM users ORDER BY id LIMIT 1;\" 2>/dev/null || true",
    )
    .await
}

async fn read_xui_sqlite_user_password(
    app: &AppHandle,
    server: &ServerConfig,
) -> anyhow::Result<Option<String>> {
    let value = read_xui_sqlite_value(
        app,
        server,
        "sqlite3 /etc/x-ui/x-ui.db \"SELECT password FROM users ORDER BY id LIMIT 1;\" 2>/dev/null || true",
    )
    .await?;
    // Newer 3x-ui stores a bcrypt hash here — skip it, it can't be used as a login password.
    Ok(value.filter(|v| !v.starts_with("$2")))
}

async fn read_xui_sqlite_value(
    app: &AppHandle,
    server: &ServerConfig,
    command: &str,
) -> anyhow::Result<Option<String>> {
    let output = ssh::execute_combined(app, server, command, 60).await?;
    Ok(output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned))
}

fn merge_panel_setup_info(base: &mut PanelSetupInfo, candidate: PanelSetupInfo) {
    if base.username == "admin" && candidate.username != "admin" {
        base.username = candidate.username;
    }
    if base.port == 65333 && candidate.port != 65333 {
        base.port = candidate.port;
    }
    if base.password.is_empty() && !candidate.password.is_empty() {
        base.password = candidate.password;
    }
    if base.web_base_path.is_empty() && !candidate.web_base_path.is_empty() {
        base.web_base_path = candidate.web_base_path;
    }
}

fn panel_url_from_setup_info(host: &str, port: u16, web_base_path: &str) -> String {
    let base_path = normalize_panel_base_path(web_base_path);
    if base_path.is_empty() {
        format!("http://{host}:{port}")
    } else {
        format!("http://{host}:{port}{base_path}")
    }
}

fn normalize_panel_base_path(value: &str) -> String {
    let trimmed = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

fn value_after_separator(line: &str) -> Option<String> {
    line.split_once(':')
        .or_else(|| line.split_once('='))
        .map(|(_, value)| {
            strip_ansi_codes(value.trim())
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn strip_ansi_codes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn first_u16(line: &str) -> Option<u16> {
    line.split(|character: char| !character.is_ascii_digit())
        .filter(|value| !value.is_empty())
        .find_map(|value| value.parse::<u16>().ok())
}

/// Extract the latest timestamp (epoch ms) for each server from a cached
/// metrics JSON object `{ serverId: [ { timestamp, ... }, ... ] }`.
fn latest_timestamps(cache: &Value) -> HashMap<String, i64> {
    let mut since = HashMap::new();
    let Some(obj) = cache.as_object() else { return since };
    for (server_id, history) in obj {
        let Some(arr) = history.as_array() else { continue };
        let latest_ms = arr
            .iter()
            .filter_map(|point| {
                point
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                    .map(|dt| dt.timestamp_millis())
            })
            .max()
            .unwrap_or(0);
        since.insert(server_id.clone(), latest_ms);
    }
    since
}

/// Append new points from `delta` into `base`.  Points are appended without
/// deduplication — the frontend's normalizeHistory sorts and deduplicates.
fn merge_cache_delta(base: &mut Value, delta: Value) {
    let (Some(base_obj), Some(delta_obj)) = (base.as_object_mut(), delta.as_object()) else {
        return;
    };
    for (server_id, new_points) in delta_obj {
        let Some(new_arr) = new_points.as_array() else { continue };
        if new_arr.is_empty() {
            continue;
        }
        let entry = base_obj
            .entry(server_id.clone())
            .or_insert_with(|| Value::Array(vec![]));
        if let Some(existing) = entry.as_array_mut() {
            existing.extend(new_arr.iter().cloned());
        }
    }
}

fn expand_public_key_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("SSH public key path is required".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let base_dirs = directories::BaseDirs::new()
            .ok_or_else(|| "unable to resolve user directories".to_string())?;
        return Ok(base_dirs.home_dir().join(rest));
    }
    Ok(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_panel_setup_info_with_web_base_path() {
        let info = parse_panel_setup_info(
            r#"
Username: admin
Password: secret
Port: 65333
WebBasePath: abc123/
"#,
            "cli",
        );

        assert_eq!(info.username, "admin");
        assert_eq!(info.password, "secret");
        assert_eq!(info.port, 65333);
        assert_eq!(info.web_base_path, "/abc123");
        assert_eq!(info.source, "cli");
    }

    #[test]
    fn builds_panel_url_with_normalized_base_path() {
        assert_eq!(
            panel_url_from_setup_info("example.com", 65333, "/panel-base/"),
            "http://example.com:65333/panel-base"
        );
        assert_eq!(
            panel_url_from_setup_info("example.com", 65333, ""),
            "http://example.com:65333"
        );
    }

    #[test]
    fn strips_ansi_codes_from_panel_values() {
        assert_eq!(
            value_after_separator("webBasePath: /abc/ \u{1b}[0m").as_deref(),
            Some("/abc/")
        );
    }
}

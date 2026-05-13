use crate::{
    config::{
        config_path, delete_server as delete_server_config, find_server, load_config, save_config,
        set_poll_interval as set_poll_interval_config, set_theme as set_theme_config,
        upsert_server as upsert_server_config, AppConfig, ServerConfig,
    },
    metrics::{collect, ServerMetrics},
    ssh::{
        self, delete_bastion_password as delete_bastion_password_secret, delete_key_passphrase,
        delete_password, ping, save_bastion_password as save_bastion_password_secret,
        save_key_passphrase, save_password, PingResult,
    },
    three_x_ui::{self, ThreeXClient, ThreeXInbound},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
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
pub fn delete_server(app: AppHandle, server_id: String) -> Result<AppConfig, String> {
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
    if let Some(password) = bastion_password.filter(|value| !value.is_empty()) {
        save_bastion_password_secret(&app, &server, &password)
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
    if let Some(existing) = config
        .servers
        .iter_mut()
        .find(|existing| existing.id == server_id)
    {
        existing.panel_url = Some(format!("http://{}:{}", existing.host, info.port));
        existing.panel_user = Some(info.username.clone());
        server = existing.clone();
    }
    save_config(&config).map_err(|error| error.to_string())?;
    if !info.password.is_empty() {
        three_x_ui::save_credentials(&app, &server, &info.username, &info.password)
            .await
            .map_err(|error| error.to_string())?;
    }
    let _ = app.emit("servers-changed", ());

    Ok(info)
}

pub async fn get_panel_setup_info(
    app: &AppHandle,
    server: &ServerConfig,
) -> anyhow::Result<PanelSetupInfo> {
    let output = ssh::execute_combined(app, server, "x-ui settings 2>&1 || true", 60).await?;
    let mut info = parse_panel_setup_info(&output, "cli");
    if !info.password.is_empty() {
        return Ok(info);
    }

    let sqlite_username = read_xui_sqlite_setting(app, server, "webUsername").await?;
    let sqlite_password = read_xui_sqlite_setting(app, server, "webPassword").await?;
    let sqlite_port = read_xui_sqlite_setting(app, server, "webPort").await?;
    if let Some(username) = sqlite_username {
        info.username = username;
    }
    if let Some(port) = sqlite_port.and_then(|value| value.parse::<u16>().ok()) {
        info.port = port;
    }
    if let Some(password) = sqlite_password {
        info.password = password;
        info.source = "sqlite".to_string();
        return Ok(info);
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

    info.source = "default".to_string();
    Ok(info)
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

fn parse_panel_setup_info(output: &str, source: &str) -> PanelSetupInfo {
    let mut port = None;
    let mut username = None;
    let mut password = None;

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
    }

    PanelSetupInfo {
        port: port.unwrap_or(65333),
        username: username.unwrap_or_else(|| "admin".to_string()),
        password: password.unwrap_or_default(),
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
    let output = ssh::execute_combined(app, server, &command, 60).await?;
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
}

fn value_after_separator(line: &str) -> Option<String> {
    line.split_once(':')
        .or_else(|| line.split_once('='))
        .map(|(_, value)| {
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn first_u16(line: &str) -> Option<u16> {
    line.split(|character: char| !character.is_ascii_digit())
        .filter(|value| !value.is_empty())
        .find_map(|value| value.parse::<u16>().ok())
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

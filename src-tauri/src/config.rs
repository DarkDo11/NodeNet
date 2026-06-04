use anyhow::{bail, Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub country: String,
    pub panel_url: Option<String>,
    pub panel_user: Option<String>,
    pub ssh_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_ssh_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_passphrase: Option<String>,
    #[serde(default)]
    pub ssl_verify: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BastionConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    pub poll_interval_sec: u64,
    pub servers: Vec<ServerConfig>,
    #[serde(default)]
    pub bastions: Vec<BastionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_bastion_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            poll_interval_sec: 10,
            servers: Vec::new(),
            bastions: Vec::new(),
            monitor_server_id: None,
            monitor_bastion_id: None,
        }
    }
}

fn default_theme() -> String {
    "dark".to_string()
}

pub fn config_dir() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("unable to resolve user directories")?;
    Ok(base_dirs.data_dir().join("NodeNet"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

fn legacy_config_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("unable to resolve user directories")?;
    Ok(base_dirs.data_dir().join("vpnctrl").join("config.json"))
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;

    if !path.exists() {
        let legacy_path = legacy_config_path()?;
        if legacy_path.exists() {
            let raw = fs::read_to_string(&legacy_path)
                .with_context(|| format!("failed to read config at {}", legacy_path.display()))?;
            let config = serde_json::from_str::<AppConfig>(&raw)
                .with_context(|| format!("failed to parse config at {}", legacy_path.display()))?;
            save_config(&config)?;
            return Ok(config);
        }

        let config = AppConfig::default();
        save_config(&config)?;
        return Ok(config);
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    let config = serde_json::from_str::<AppConfig>(&raw)
        .with_context(|| format!("failed to parse config at {}", path.display()))?;

    Ok(config)
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let directory = config_dir()?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create config directory {}", directory.display()))?;

    let path = config_path()?;
    let raw = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &raw)
        .with_context(|| format!("failed to write config {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to commit config {}", path.display()))
}

pub fn upsert_server(mut server: ServerConfig) -> Result<AppConfig> {
    let mut config = load_config()?;
    if server.id.trim().is_empty() {
        server.id = uuid::Uuid::new_v4().simple().to_string();
    }

    if let Some(existing) = config
        .servers
        .iter_mut()
        .find(|existing| existing.id == server.id)
    {
        *existing = server;
    } else {
        config.servers.push(server);
    }

    save_config(&config)?;
    Ok(config)
}

pub fn delete_server(server_id: &str) -> Result<AppConfig> {
    let mut config = load_config()?;
    config.servers.retain(|server| server.id != server_id);
    if config.monitor_server_id.as_deref() == Some(server_id) {
        config.monitor_server_id = None;
    }
    save_config(&config)?;
    Ok(config)
}

pub fn upsert_bastion(mut bastion: BastionConfig) -> Result<AppConfig> {
    let mut config = load_config()?;
    if bastion.id.trim().is_empty() {
        bastion.id = uuid::Uuid::new_v4().simple().to_string();
    }

    if let Some(existing) = config
        .bastions
        .iter_mut()
        .find(|existing| existing.id == bastion.id)
    {
        *existing = bastion;
    } else {
        config.bastions.push(bastion);
    }

    save_config(&config)?;
    Ok(config)
}

pub fn delete_bastion(bastion_id: &str) -> Result<AppConfig> {
    let mut config = load_config()?;
    config.bastions.retain(|bastion| bastion.id != bastion_id);
    if config.monitor_bastion_id.as_deref() == Some(bastion_id) {
        config.monitor_bastion_id = None;
    }
    save_config(&config)?;
    Ok(config)
}

pub fn set_poll_interval(seconds: u64) -> Result<AppConfig> {
    let mut config = load_config()?;
    config.poll_interval_sec = seconds.clamp(2, 120);
    save_config(&config)?;
    Ok(config)
}

pub fn set_theme(theme: String) -> Result<AppConfig> {
    let mut config = load_config()?;
    match theme.as_str() {
        "dark" | "purple-dark" | "green-dark" | "full-dark" | "system" | "contrast" => {
            config.theme = theme;
        }
        other => bail!(
            "unknown theme '{other}', allowed: dark, purple-dark, green-dark, full-dark, system, contrast"
        ),
    }
    save_config(&config)?;
    Ok(config)
}

pub fn set_monitor_server(server_id: Option<String>) -> Result<AppConfig> {
    set_monitor_target(server_id, None)
}

pub fn set_monitor_target(
    server_id: Option<String>,
    bastion_id: Option<String>,
) -> Result<AppConfig> {
    let mut config = load_config()?;
    let server_id = server_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let bastion_id = bastion_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if server_id.is_some() && bastion_id.is_some() {
        bail!("monitor target must be either server or bastion");
    }

    if let Some(server_id) = &server_id {
        if !config.servers.iter().any(|server| &server.id == server_id) {
            bail!("monitor server '{server_id}' was not found");
        }
    }
    if let Some(bastion_id) = &bastion_id {
        if !config
            .bastions
            .iter()
            .any(|bastion| &bastion.id == bastion_id)
        {
            bail!("monitor bastion '{bastion_id}' was not found");
        }
    }

    config.monitor_server_id = server_id;
    config.monitor_bastion_id = bastion_id;
    save_config(&config)?;
    Ok(config)
}

pub fn find_server(server_id: &str) -> Result<ServerConfig> {
    let config = load_config()?;
    config
        .servers
        .into_iter()
        .find(|server| server.id == server_id)
        .with_context(|| format!("server '{server_id}' was not found"))
}

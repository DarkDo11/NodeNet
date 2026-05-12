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
pub struct AppConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    pub poll_interval_sec: u64,
    pub servers: Vec<ServerConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            poll_interval_sec: 10,
            servers: Vec::new(),
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
    fs::write(&path, raw).with_context(|| format!("failed to write config {}", path.display()))?;

    Ok(())
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
        "dark" | "system" | "contrast" => {
            config.theme = theme;
        }
        other => bail!("unknown theme '{other}', allowed: dark, system, contrast"),
    }
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

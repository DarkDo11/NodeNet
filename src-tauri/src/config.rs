use anyhow::{Context, Result};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub poll_interval_sec: u64,
    pub servers: Vec<ServerConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_interval_sec: 10,
            servers: vec![ServerConfig {
                id: "de-1".to_string(),
                name: "Germany 1".to_string(),
                host: "1.2.3.4".to_string(),
                ssh_port: 22,
                ssh_user: "root".to_string(),
                country: "DE".to_string(),
                panel_url: Some("https://panel.example.com".to_string()),
                panel_user: Some("admin".to_string()),
                ssh_key_path: None,
            }],
        }
    }
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

pub fn find_server(server_id: &str) -> Result<ServerConfig> {
    let config = load_config()?;
    config
        .servers
        .into_iter()
        .find(|server| server.id == server_id)
        .with_context(|| format!("server '{server_id}' was not found"))
}

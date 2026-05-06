use crate::{config, ssh, three_x_ui};
use anyhow::{Context, Result};
use directories::BaseDirs;
use serde_json::Value;
use std::{fs, path::PathBuf};
use tauri::AppHandle;

const SSH_SECRET_FIELDS: &[&str] = &["sshPassword", "sshPass", "ssh_password", "password"];
const SSH_KEY_SECRET_FIELDS: &[&str] = &["sshKeyPassphrase", "ssh_key_passphrase"];
const PANEL_SECRET_FIELDS: &[&str] = &[
    "panelPassword",
    "panelPass",
    "threeXUiPassword",
    "threeXPassword",
    "xuiPassword",
    "xuiPass",
];

pub async fn migrate_plaintext_config_secrets(app: &AppHandle) -> Result<()> {
    let path = config::config_path()?;
    if !path.exists() {
        let legacy_path = legacy_config_path()?;
        if legacy_path.exists() {
            migrate_file(app, legacy_path).await?;
            let _ = config::load_config()?;
            return Ok(());
        }
    }

    let _ = config::load_config()?;
    migrate_file(app, path).await
}

async fn migrate_file(app: &AppHandle, path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut value = serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let mut changed = false;

    if let Some(servers) = value.get_mut("servers").and_then(Value::as_array_mut) {
        for server_value in servers {
            let Some(server_object) = server_value.as_object_mut() else {
                continue;
            };

            let server = serde_json::from_value::<config::ServerConfig>(Value::Object(
                server_object.clone(),
            ))
            .context("failed to parse server while migrating secrets")?;

            if let Some(password) = take_first_string(server_object, SSH_SECRET_FIELDS) {
                ssh::save_password(app, &server, &password).await?;
                changed = true;
            }

            if let Some(passphrase) = take_first_string(server_object, SSH_KEY_SECRET_FIELDS) {
                ssh::save_key_passphrase(app, &server, &passphrase).await?;
                changed = true;
            }

            if let Some(password) = take_first_string(server_object, PANEL_SECRET_FIELDS) {
                let username = server
                    .panel_user
                    .clone()
                    .filter(|user| !user.trim().is_empty())
                    .unwrap_or_else(|| "admin".to_string());
                three_x_ui::save_credentials(app, &server, &username, &password).await?;
                changed = true;
            }
        }
    }

    if changed {
        let config = serde_json::from_value::<config::AppConfig>(value)
            .context("failed to sanitize config")?;
        config::save_config(&config)?;
    }

    remove_legacy_plaintext_file_if_empty()?;
    Ok(())
}

fn take_first_string(
    object: &mut serde_json::Map<String, Value>,
    fields: &[&str],
) -> Option<String> {
    let mut found = None;
    for field in fields {
        if let Some(value) = object.remove(*field) {
            if found.is_none() {
                found = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
            }
        }
    }
    found
}

fn remove_legacy_plaintext_file_if_empty() -> Result<()> {
    let legacy_path = legacy_config_path()?;
    if !legacy_path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&legacy_path)
        .with_context(|| format!("failed to read {}", legacy_path.display()))?;
    if raw.contains("sshPassword")
        || raw.contains("panelPassword")
        || raw.contains("sshKeyPassphrase")
        || raw.contains("threeXUiPassword")
        || raw.contains("\"password\"")
    {
        let _ = fs::rename(&legacy_path, legacy_path.with_extension("json.migrated"));
    }

    Ok(())
}

fn legacy_config_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("unable to resolve user directories")?;
    Ok(base_dirs.data_dir().join("vpnctrl").join("config.json"))
}

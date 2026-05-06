use crate::{config::ServerConfig, keychain};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::{net::TcpStream, process::Command, time::timeout};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.ssh";
const LEGACY_KEYCHAIN_SERVICE: &str = "vpnctrl.ssh";
const SSH_CONNECT_TIMEOUT_SECS: u64 = 8;
const SSH_COMMAND_TIMEOUT_SECS: u64 = 28;
const SSH_RETRY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub server_id: String,
    pub latency_ms: Option<u128>,
    pub status: String,
    pub message: String,
    pub checked_at: DateTime<Utc>,
}

pub fn keychain_account(server: &ServerConfig) -> String {
    format!("{}@{}:{}", server.ssh_user, server.host, server.ssh_port)
}

pub async fn save_password(app: &AppHandle, server: &ServerConfig, password: &str) -> Result<()> {
    let account = keychain_account(server);
    keychain::save_password(app, KEYCHAIN_SERVICE, &account, password).await
}

pub async fn delete_password(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    let account = keychain_account(server);
    let primary_result = keychain::delete_password(app, KEYCHAIN_SERVICE, &account).await;
    let legacy_result = keychain::delete_password(app, LEGACY_KEYCHAIN_SERVICE, &account).await;

    match (primary_result, legacy_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(primary_error), Err(_legacy_error)) => Err(primary_error),
    }
}

pub async fn read_password(app: &AppHandle, server: &ServerConfig) -> Result<Option<String>> {
    let account = keychain_account(server);
    if let Some(password) = keychain::read_password(app, KEYCHAIN_SERVICE, &account).await? {
        return Ok(Some(password));
    }

    keychain::read_password(app, LEGACY_KEYCHAIN_SERVICE, &account).await
}

pub async fn ping(server: &ServerConfig) -> PingResult {
    let checked_at = Utc::now();
    let started = Instant::now();
    let address = (server.host.as_str(), server.ssh_port);

    match timeout(Duration::from_secs(3), TcpStream::connect(address)).await {
        Ok(Ok(_stream)) => {
            let latency_ms = started.elapsed().as_millis();
            let status = if latency_ms > 1_000 {
                "warning"
            } else {
                "online"
            };

            PingResult {
                server_id: server.id.clone(),
                latency_ms: Some(latency_ms),
                status: status.to_string(),
                message: "tcp ok".to_string(),
                checked_at,
            }
        }
        Ok(Err(error)) => PingResult {
            server_id: server.id.clone(),
            latency_ms: None,
            status: "offline".to_string(),
            message: error.to_string(),
            checked_at,
        },
        Err(_) => PingResult {
            server_id: server.id.clone(),
            latency_ms: None,
            status: "offline".to_string(),
            message: "connection timeout".to_string(),
            checked_at,
        },
    }
}

pub async fn execute(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
) -> Result<String> {
    retry_ssh_operation(|| async { execute_once(app, server, remote_command).await }).await
}

async fn execute_once(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
) -> Result<String> {
    let password = if server.ssh_key_path.is_none() {
        read_password(app, server).await?
    } else {
        None
    };
    let askpass_path = if password.is_some() {
        Some(create_askpass_script()?)
    } else {
        None
    };

    let mut command = Command::new("ssh");
    command
        .kill_on_drop(true)
        .arg("-p")
        .arg(server.ssh_port.to_string())
        .arg("-o")
        .arg("BatchMode=no")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("PreferredAuthentications=publickey,password");

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    if let (Some(password), Some(askpass_path)) = (&password, &askpass_path) {
        command
            .env("SSH_ASKPASS", askpass_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", ":0")
            .env("NODENET_SSH_PASSWORD", password);
    }

    command
        .arg(format!("{}@{}", server.ssh_user, server.host))
        .arg(remote_command);

    let output = timeout(
        Duration::from_secs(SSH_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .context("ssh command timed out")?
    .context("failed to execute ssh")?;

    if let Some(path) = askpass_path {
        let _ = fs::remove_file(path);
    }

    if !output.status.success() {
        return Err(anyhow!(
            "ssh command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn download_file(
    app: &AppHandle,
    server: &ServerConfig,
    remote_path: &str,
    local_path: &Path,
) -> Result<()> {
    retry_ssh_operation(|| async { download_file_once(app, server, remote_path, local_path).await })
        .await
}

async fn download_file_once(
    app: &AppHandle,
    server: &ServerConfig,
    remote_path: &str,
    local_path: &Path,
) -> Result<()> {
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create backup directory {}", parent.display()))?;
    }

    let password = if server.ssh_key_path.is_none() {
        read_password(app, server).await?
    } else {
        None
    };
    let askpass_path = if password.is_some() {
        Some(create_askpass_script()?)
    } else {
        None
    };
    let batch_path = std::env::temp_dir().join(format!("nodenet-sftp-{}.batch", Uuid::new_v4()));
    fs::write(
        &batch_path,
        format!(
            "get {} {}\n",
            sftp_quote(remote_path),
            sftp_quote(&local_path.display().to_string())
        ),
    )
    .with_context(|| format!("failed to write sftp batch {}", batch_path.display()))?;

    let mut command = Command::new("sftp");
    command
        .kill_on_drop(true)
        .arg("-P")
        .arg(server.ssh_port.to_string())
        .arg("-o")
        .arg("BatchMode=no")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-b")
        .arg(&batch_path);

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    if let (Some(password), Some(askpass_path)) = (&password, &askpass_path) {
        command
            .env("SSH_ASKPASS", askpass_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", ":0")
            .env("NODENET_SSH_PASSWORD", password);
    }

    command.arg(format!("{}@{}", server.ssh_user, server.host));

    let output = timeout(
        Duration::from_secs(SSH_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .context("sftp download timed out")?
    .context("failed to execute sftp")?;

    if let Some(path) = askpass_path {
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_file(batch_path);

    if !output.status.success() {
        return Err(anyhow!(
            "sftp download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

async fn retry_ssh_operation<F, Fut, T>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut delay = Duration::from_millis(450);
    let mut last_error = None;

    for attempt in 0..SSH_RETRY_ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if attempt + 1 < SSH_RETRY_ATTEMPTS => {
                last_error = Some(error);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(4));
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("ssh operation failed")))
}

fn create_askpass_script() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("nodenet-askpass-{}.sh", Uuid::new_v4()));
    fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$NODENET_SSH_PASSWORD\"\n",
    )
    .with_context(|| format!("failed to write askpass script {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions)?;
    }

    Ok(path)
}

fn sftp_quote(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest).display().to_string();
        }
    }

    path.to_string()
}

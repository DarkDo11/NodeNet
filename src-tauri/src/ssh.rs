use crate::{config::ServerConfig, keychain, util::expand_tilde};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::Mutex;
use tokio::{net::TcpStream, process::Command, time::timeout};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.ssh";
const KEYCHAIN_KEY_SERVICE: &str = "nodenet.ssh.key";
const LEGACY_KEYCHAIN_SERVICE: &str = "vpnctrl.ssh";
const SSH_CONNECT_TIMEOUT_SECS: u64 = 8;
const SSH_COMMAND_TIMEOUT_SECS: u64 = 28;
const SSH_RETRY_ATTEMPTS: usize = 3;
const SSH_IDLE_TIMEOUT_SECS: u64 = 120;

type SshConnectionPool = Mutex<HashMap<String, Arc<Mutex<SshConnection>>>>;

static SSH_POOL: LazyLock<SshConnectionPool> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub struct SshConnection {
    control_path: PathBuf,
    last_used: Instant,
}

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

pub async fn save_key_passphrase(
    app: &AppHandle,
    server: &ServerConfig,
    passphrase: &str,
) -> Result<()> {
    let account = keychain_account(server);
    keychain::save_password(app, KEYCHAIN_KEY_SERVICE, &account, passphrase).await
}

pub async fn delete_key_passphrase(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    let account = keychain_account(server);
    keychain::delete_password(app, KEYCHAIN_KEY_SERVICE, &account).await
}

pub async fn read_key_passphrase(app: &AppHandle, server: &ServerConfig) -> Result<Option<String>> {
    let account = keychain_account(server);
    if let Some(passphrase) = keychain::read_password(app, KEYCHAIN_KEY_SERVICE, &account).await? {
        return Ok(Some(passphrase));
    }

    Ok(server
        .ssh_key_passphrase
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
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
    let connection = get_or_create_connection(app, server).await?;
    let control_path = {
        let mut connection = connection.lock().await;
        connection.last_used = Instant::now();
        connection.control_path.clone()
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
        .arg("PreferredAuthentications=publickey,password")
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()));

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
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
    let connection = get_or_create_connection(app, server).await?;
    let control_path = {
        let mut connection = connection.lock().await;
        connection.last_used = Instant::now();
        connection.control_path.clone()
    };

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create backup directory {}", parent.display()))?;
    }

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
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-b")
        .arg(&batch_path);

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    command.arg(format!("{}@{}", server.ssh_user, server.host));

    let output = timeout(
        Duration::from_secs(SSH_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .context("sftp download timed out")?
    .context("failed to execute sftp")?;

    let _ = fs::remove_file(batch_path);

    if !output.status.success() {
        return Err(anyhow!(
            "sftp download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

async fn get_or_create_connection(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Arc<Mutex<SshConnection>>> {
    let account = keychain_account(server);
    let existing = { SSH_POOL.lock().await.get(&account).cloned() };
    if let Some(connection) = existing {
        let alive = {
            let connection = connection.lock().await;
            connection.last_used.elapsed() < Duration::from_secs(SSH_IDLE_TIMEOUT_SECS)
                && is_connection_alive(server, &connection.control_path).await
        };

        if alive {
            return Ok(connection);
        }

        let control_path = connection.lock().await.control_path.clone();
        close_master(server, &control_path).await;
        SSH_POOL.lock().await.remove(&account);
    }

    let control_path = std::env::temp_dir().join(format!("nodenet-ssh-{}.sock", Uuid::new_v4()));
    open_master(app, server, &control_path).await?;
    let connection = Arc::new(Mutex::new(SshConnection {
        control_path,
        last_used: Instant::now(),
    }));
    SSH_POOL
        .lock()
        .await
        .insert(account, Arc::clone(&connection));
    Ok(connection)
}

async fn open_master(app: &AppHandle, server: &ServerConfig, control_path: &Path) -> Result<()> {
    let secret = if server.ssh_key_path.is_some() {
        read_key_passphrase(app, server).await?
    } else {
        read_password(app, server).await?
    };
    let askpass_path = if let Some(secret) = &secret {
        Some(create_askpass_script(secret)?)
    } else {
        None
    };

    let mut command = Command::new("ssh");
    command
        .kill_on_drop(true)
        .arg("-M")
        .arg("-N")
        .arg("-f")
        .arg("-p")
        .arg(server.ssh_port.to_string())
        .arg("-o")
        .arg("BatchMode=no")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg(format!("ControlPersist={SSH_IDLE_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
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

    if let Some(askpass_path) = &askpass_path {
        command
            .env("SSH_ASKPASS", askpass_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", ":0");
    }

    command.arg(format!("{}@{}", server.ssh_user, server.host));

    let output = timeout(
        Duration::from_secs(SSH_CONNECT_TIMEOUT_SECS + 4),
        command.output(),
    )
    .await
    .context("ssh master connection timed out")?
    .context("failed to start ssh master connection")?;

    if let Some(path) = askpass_path {
        let _ = fs::remove_file(path);
    }

    if !output.status.success() {
        return Err(anyhow!(
            "ssh master connection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

async fn is_connection_alive(server: &ServerConfig, control_path: &Path) -> bool {
    let Ok(output) = timeout(Duration::from_secs(2), async {
        let mut command = Command::new("ssh");
        command
            .arg("-O")
            .arg("check")
            .arg("-p")
            .arg(server.ssh_port.to_string())
            .arg("-o")
            .arg(format!("ControlPath={}", control_path.display()))
            .arg(format!("{}@{}", server.ssh_user, server.host));
        command.output().await
    })
    .await
    else {
        return false;
    };

    output
        .map(|output| output.status.success())
        .unwrap_or(false)
}

async fn close_master(server: &ServerConfig, control_path: &Path) {
    let _ = timeout(Duration::from_secs(2), async {
        let mut command = Command::new("ssh");
        command
            .arg("-O")
            .arg("exit")
            .arg("-p")
            .arg(server.ssh_port.to_string())
            .arg("-o")
            .arg(format!("ControlPath={}", control_path.display()))
            .arg(format!("{}@{}", server.ssh_user, server.host));
        command.output().await
    })
    .await;
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
            Err(error) if is_auth_error(&error) => {
                return Err(anyhow!(
                    "SSH authentication failed. Check password in Keychain → Settings."
                ));
            }
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

fn is_auth_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("authentication")
        || message.contains("permission denied")
        || message.contains("too many authentication failures")
        || message.contains("publickey,password")
}

fn create_askpass_script(password: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("nodenet-askpass-{}.sh", Uuid::new_v4()));
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' {}\n",
            shell_single_quote(password)
        ),
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

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

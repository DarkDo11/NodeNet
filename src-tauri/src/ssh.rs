use crate::{config::ServerConfig, keychain, util::expand_tilde};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    net::TcpStream,
    process::Command,
    sync::Mutex,
    time::timeout,
};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.ssh";
const KEYCHAIN_KEY_SERVICE: &str = "nodenet.ssh.key";
const KEYCHAIN_BASTION_SERVICE: &str = "nodenet.ssh.bastion";
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutputEvent {
    pub session_id: String,
    pub server_id: String,
    pub line: String,
    pub done: bool,
}

pub fn keychain_account(server: &ServerConfig) -> String {
    format!("{}@{}:{}", server.ssh_user, server.host, server.ssh_port)
}

pub fn bastion_keychain_account(server: &ServerConfig) -> Option<String> {
    let host = server.bastion_host.as_ref()?.trim();
    if host.is_empty() {
        return None;
    }

    let user = server
        .bastion_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server.ssh_user.as_str());
    let port = server.bastion_port.unwrap_or(22);
    Some(format!("{user}@{host}:{port}"))
}

pub fn cleanup_stale_sockets() {
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("nodenet-ssh-") && name.ends_with(".sock") {
                let _ = std::fs::remove_file(entry.path());
            }
            if name.starts_with("nodenet-askpass-") && name.ends_with(".sh") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
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

pub async fn save_bastion_password(
    app: &AppHandle,
    server: &ServerConfig,
    password: &str,
) -> Result<()> {
    let account = bastion_keychain_account(server).context("bastion host is not configured")?;
    keychain::save_password(app, KEYCHAIN_BASTION_SERVICE, &account, password).await
}

pub async fn delete_bastion_password(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    let account = bastion_keychain_account(server).context("bastion host is not configured")?;
    keychain::delete_password(app, KEYCHAIN_BASTION_SERVICE, &account).await
}

pub async fn read_bastion_password(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<String>> {
    let Some(account) = bastion_keychain_account(server) else {
        return Ok(None);
    };

    keychain::read_password(app, KEYCHAIN_BASTION_SERVICE, &account).await
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
    let bastion_host = server
        .bastion_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let address = if let Some(host) = bastion_host {
        (host, server.bastion_port.unwrap_or(22))
    } else {
        (server.host.as_str(), server.ssh_port)
    };

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
                message: if bastion_host.is_some() {
                    "bastion tcp ok".to_string()
                } else {
                    "tcp ok".to_string()
                },
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

pub async fn execute_combined(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
    timeout_secs: u64,
) -> Result<String> {
    retry_ssh_operation(|| async {
        execute_once_with_options(app, server, remote_command, timeout_secs, true).await
    })
    .await
}

pub async fn execute_streaming_combined(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
    session_id: &str,
) -> Result<()> {
    let result = execute_streaming_once(app, server, remote_command, session_id).await;
    match &result {
        Ok(()) => emit_command_output(app, server, session_id, "", true),
        Err(error) => emit_command_output(app, server, session_id, &error.to_string(), true),
    }
    result
}

async fn execute_once(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
) -> Result<String> {
    execute_once_with_options(app, server, remote_command, SSH_COMMAND_TIMEOUT_SECS, false).await
}

async fn execute_once_with_options(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
    timeout_secs: u64,
    include_stderr: bool,
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

    apply_bastion_proxy(&mut command, server);

    command
        .arg(format!("{}@{}", server.ssh_user, server.host))
        .arg(remote_command);

    let output = timeout(Duration::from_secs(timeout_secs), command.output())
        .await
        .context("ssh command timed out")?
        .context("failed to execute ssh")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if include_stderr && !stderr.trim().is_empty() {
        format!("{stdout}{stderr}")
    } else {
        stdout.to_string()
    };

    if !output.status.success() {
        return Err(anyhow!("ssh command failed: {}", combined.trim()));
    }

    Ok(combined)
}

async fn execute_streaming_once(
    app: &AppHandle,
    server: &ServerConfig,
    remote_command: &str,
    session_id: &str,
) -> Result<()> {
    let connection = get_or_create_connection(app, server).await?;
    let control_path = {
        let mut connection = connection.lock().await;
        connection.last_used = Instant::now();
        connection.control_path.clone()
    };

    let mut command = Command::new("ssh");
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

    apply_bastion_proxy(&mut command, server);

    command
        .arg(format!("{}@{}", server.ssh_user, server.host))
        .arg(remote_command);

    let mut child = command.spawn().context("failed to execute ssh")?;
    let stdout = child
        .stdout
        .take()
        .context("failed to capture ssh stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture ssh stderr")?;
    let stdout_task = tokio::spawn(stream_command_output(
        app.clone(),
        server.clone(),
        session_id.to_string(),
        stdout,
    ));
    let stderr_task = tokio::spawn(stream_command_output(
        app.clone(),
        server.clone(),
        session_id.to_string(),
        stderr,
    ));

    let status = child.wait().await.context("failed to wait for ssh")?;
    stdout_task
        .await
        .context("ssh stdout stream task failed")??;
    stderr_task
        .await
        .context("ssh stderr stream task failed")??;

    if !status.success() {
        bail!("ssh command failed with status {status}");
    }

    Ok(())
}

async fn stream_command_output<R>(
    app: AppHandle,
    server: ServerConfig,
    session_id: String,
    stream: R,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("failed to read ssh output")?
    {
        emit_command_output(&app, &server, &session_id, &line, false);
    }
    Ok(())
}

fn emit_command_output(
    app: &AppHandle,
    server: &ServerConfig,
    session_id: &str,
    line: &str,
    done: bool,
) {
    let _ = app.emit(
        "command-output",
        CommandOutputEvent {
            session_id: session_id.to_string(),
            server_id: server.id.clone(),
            line: line.to_string(),
            done,
        },
    );
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

    apply_bastion_proxy(&mut command, server);

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
    let account = connection_pool_account(server);
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

fn connection_pool_account(server: &ServerConfig) -> String {
    match bastion_route_id(server) {
        Some(route) => format!("{} via {route}", keychain_account(server)),
        None => keychain_account(server),
    }
}

async fn open_master(app: &AppHandle, server: &ServerConfig, control_path: &Path) -> Result<()> {
    let target_secret = if server.ssh_key_path.is_some() {
        read_key_passphrase(app, server).await?
    } else {
        read_password(app, server).await?
    };
    let bastion_secret = read_bastion_password(app, server).await?;
    let askpass_path =
        create_askpass_script(target_secret.as_deref(), bastion_secret.as_deref(), server)?;

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

    apply_bastion_proxy(&mut command, server);

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
            .arg(format!("ControlPath={}", control_path.display()));
        apply_bastion_proxy(&mut command, server);
        command.arg(format!("{}@{}", server.ssh_user, server.host));
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
            .arg(format!("ControlPath={}", control_path.display()));
        apply_bastion_proxy(&mut command, server);
        command.arg(format!("{}@{}", server.ssh_user, server.host));
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

fn create_askpass_script(
    target_secret: Option<&str>,
    bastion_secret: Option<&str>,
    server: &ServerConfig,
) -> Result<Option<PathBuf>> {
    if target_secret.is_none() && bastion_secret.is_none() {
        return Ok(None);
    }

    let path = std::env::temp_dir().join(format!("nodenet-askpass-{}.sh", Uuid::new_v4()));
    let bastion_account = bastion_keychain_account(server).unwrap_or_default();
    let bastion_host = server.bastion_host.as_deref().unwrap_or_default();
    let target_secret = target_secret.unwrap_or_default();
    let bastion_secret = bastion_secret.unwrap_or(target_secret);
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprompt=\"$1\"\ncase \"$prompt\" in\n  *{}*|*{}*) printf '%s\\n' {} ;;\n  *) printf '%s\\n' {} ;;\nesac\n",
            shell_case_pattern(&bastion_account),
            shell_case_pattern(bastion_host),
            shell_single_quote(bastion_secret),
            shell_single_quote(target_secret)
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

    Ok(Some(path))
}

fn apply_bastion_proxy(command: &mut Command, server: &ServerConfig) {
    if let Some(proxy_command) = bastion_proxy_command(server) {
        command
            .arg("-o")
            .arg(format!("ProxyCommand={proxy_command}"));
    } else if let Some(proxy_jump) = bastion_proxy_jump(server) {
        command.arg("-o").arg(format!("ProxyJump={proxy_jump}"));
    }
}

fn bastion_proxy_command(server: &ServerConfig) -> Option<String> {
    let key_path = server
        .bastion_ssh_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let host = bastion_host(server)?;
    let user = bastion_user(server);
    let port = server.bastion_port.unwrap_or(22);

    Some(format!(
        "ssh -W %h:%p -p {port} -o BatchMode=no -o StrictHostKeyChecking=accept-new -o PreferredAuthentications=publickey,password -i {} -o IdentitiesOnly=yes {}",
        shell_single_quote(&expand_tilde(key_path)),
        shell_single_quote(&format!("{user}@{host}")),
    ))
}

fn bastion_proxy_jump(server: &ServerConfig) -> Option<String> {
    let host = bastion_host(server)?;
    let user = bastion_user(server);
    let port = server.bastion_port.unwrap_or(22);

    Some(format!("{user}@{host}:{port}"))
}

fn bastion_route_id(server: &ServerConfig) -> Option<String> {
    bastion_proxy_command(server).or_else(|| bastion_proxy_jump(server))
}

fn bastion_host(server: &ServerConfig) -> Option<&str> {
    server
        .bastion_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn bastion_user(server: &ServerConfig) -> &str {
    server
        .bastion_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server.ssh_user.as_str())
}

fn shell_case_pattern(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' | '*' | '?' | '[' | ']' => ['\\', character],
            _ => ['\0', character],
        })
        .filter(|character| *character != '\0')
        .collect()
}

fn sftp_quote(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

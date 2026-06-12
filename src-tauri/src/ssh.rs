use crate::{config::ServerConfig, keychain, util::expand_tilde};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::{self, AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    net::TcpStream,
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
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

#[derive(Debug)]
pub struct SshTunnel {
    child: Child,
    local_port: u16,
}

#[derive(Debug)]
struct TempPathGuard {
    path: PathBuf,
}

impl TempPathGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl SshTunnel {
    pub fn local_port(&self) -> u16 {
        self.local_port
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
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
    format!(
        "{}:{}@{}:{}",
        server.id, server.ssh_user, server.host, server.ssh_port
    )
}

fn legacy_keychain_account(server: &ServerConfig) -> String {
    format!("{}@{}:{}", server.ssh_user, server.host, server.ssh_port)
}

pub fn bastion_keychain_account(server: &ServerConfig) -> Option<String> {
    bastion_keychain_account_with_id(server, true)
}

fn legacy_bastion_keychain_account(server: &ServerConfig) -> Option<String> {
    bastion_keychain_account_with_id(server, false)
}

fn bastion_keychain_account_with_id(
    server: &ServerConfig,
    include_server_id: bool,
) -> Option<String> {
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
    let account = format!("{user}@{host}:{port}");
    Some(if include_server_id {
        format!("{}:{account}", server.id)
    } else {
        account
    })
}

pub fn start_connection_reaper() {
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(SSH_IDLE_TIMEOUT_SECS)).await;
            reap_stale_connections().await;
        }
    });
}

async fn reap_stale_connections() {
    let stale_keys: Vec<String> = {
        let pool = SSH_POOL.lock().await;
        pool.iter()
            .filter_map(|(key, conn)| {
                conn.try_lock()
                    .ok()
                    .filter(|c| c.last_used.elapsed() >= Duration::from_secs(SSH_IDLE_TIMEOUT_SECS))
                    .map(|_| key.clone())
            })
            .collect()
    };

    if stale_keys.is_empty() {
        return;
    }

    let mut pool = SSH_POOL.lock().await;
    for key in &stale_keys {
        if let Some(conn) = pool.remove(key) {
            if let Ok(conn) = conn.try_lock() {
                let _ = fs::remove_file(&conn.control_path);
            }
        }
    }
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

    if let Ok(entries) = std::fs::read_dir(ssh_socket_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("c-") && name.ends_with(".sock") {
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
    let legacy_account = legacy_keychain_account(server);
    let primary_result = keychain::delete_password(app, KEYCHAIN_SERVICE, &account).await;
    let legacy_primary_result =
        keychain::delete_password(app, KEYCHAIN_SERVICE, &legacy_account).await;
    let legacy_result =
        keychain::delete_password(app, LEGACY_KEYCHAIN_SERVICE, &legacy_account).await;

    primary_result.and(legacy_primary_result).and(legacy_result)
}

pub async fn read_password(app: &AppHandle, server: &ServerConfig) -> Result<Option<String>> {
    let account = keychain_account(server);
    if let Some(password) = keychain::read_password(app, KEYCHAIN_SERVICE, &account).await? {
        return Ok(Some(password));
    }

    let legacy_account = legacy_keychain_account(server);
    if let Some(password) = keychain::read_password(app, KEYCHAIN_SERVICE, &legacy_account).await? {
        return Ok(Some(password));
    }

    keychain::read_password(app, LEGACY_KEYCHAIN_SERVICE, &legacy_account).await
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
    let primary_result = keychain::delete_password(app, KEYCHAIN_BASTION_SERVICE, &account).await;
    let legacy_result = match legacy_bastion_keychain_account(server) {
        Some(legacy_account) => {
            keychain::delete_password(app, KEYCHAIN_BASTION_SERVICE, &legacy_account).await
        }
        None => Ok(()),
    };
    primary_result.and(legacy_result)
}

pub async fn read_bastion_password(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<String>> {
    let Some(account) = bastion_keychain_account(server) else {
        return Ok(None);
    };

    if let Some(password) = keychain::read_password(app, KEYCHAIN_BASTION_SERVICE, &account).await?
    {
        return Ok(Some(password));
    }

    let Some(legacy_account) = legacy_bastion_keychain_account(server) else {
        return Ok(None);
    };
    keychain::read_password(app, KEYCHAIN_BASTION_SERVICE, &legacy_account).await
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
    let legacy_account = legacy_keychain_account(server);
    let primary_result = keychain::delete_password(app, KEYCHAIN_KEY_SERVICE, &account).await;
    let legacy_result = keychain::delete_password(app, KEYCHAIN_KEY_SERVICE, &legacy_account).await;
    primary_result.and(legacy_result)
}

pub async fn read_key_passphrase(app: &AppHandle, server: &ServerConfig) -> Result<Option<String>> {
    if let Some(passphrase) = read_saved_key_passphrase(app, server).await? {
        return Ok(Some(passphrase));
    }

    Ok(server
        .ssh_key_passphrase
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
}

pub async fn read_saved_key_passphrase(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<String>> {
    let account = keychain_account(server);
    if let Some(passphrase) = keychain::read_password(app, KEYCHAIN_KEY_SERVICE, &account).await? {
        return Ok(Some(passphrase));
    }

    let legacy_account = legacy_keychain_account(server);
    keychain::read_password(app, KEYCHAIN_KEY_SERVICE, &legacy_account).await
}

pub async fn ping(app: &AppHandle, server: &ServerConfig) -> PingResult {
    let checked_at = Utc::now();
    match ping_ms(app, server).await {
        Some(latency_ms) => {
            let status = if latency_ms > 1_000.0 {
                "warning"
            } else {
                "online"
            };
            PingResult {
                server_id: server.id.clone(),
                latency_ms: Some(latency_ms.round() as u128),
                status: status.to_string(),
                message: if has_bastion(server) {
                    "SSH route through bastion OK".to_string()
                } else {
                    "SSH OK".to_string()
                },
                checked_at,
            }
        }
        None => PingResult {
            server_id: server.id.clone(),
            latency_ms: None,
            status: "offline".to_string(),
            message: if has_bastion(server) {
                "SSH route through bastion failed".to_string()
            } else {
                "SSH failed".to_string()
            },
            checked_at,
        },
    }
}

pub async fn ping_ms(app: &AppHandle, server: &ServerConfig) -> Option<f64> {
    let started = Instant::now();
    execute(app, server, "true").await.ok()?;
    Some(round_one(started.elapsed().as_secs_f64() * 1000.0))
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub async fn open_bastion_tunnel(
    app: &AppHandle,
    server: &ServerConfig,
    target_host: &str,
    target_port: u16,
) -> Result<SshTunnel> {
    let host = bastion_host(server).context("bastion host is not configured")?;
    let user = bastion_user(server);
    let port = server.bastion_port.unwrap_or(22);
    let (local_port, port_guard) =
        reserve_local_port().context("Bastion tunnel failed: no local port")?;
    let bastion_secret = read_bastion_password(app, server).await?;
    let askpass = create_askpass_script(None, bastion_secret.as_deref(), server)
        .context("Bastion tunnel failed: could not prepare SSH authentication")?;

    let mut command = Command::new("ssh");
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .arg("-N")
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{local_port}:{target_host}:{target_port}"
        ))
        .arg("-p")
        .arg(port.to_string())
        .arg("-o")
        .arg("BatchMode=no")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
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

    if let Some(key_path) = server
        .bastion_ssh_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    if let Some(askpass) = &askpass {
        apply_askpass_env(&mut command, Some(askpass));
    }

    command.arg(format!("{user}@{host}"));

    // Release the reserved port so SSH can bind to it; the window between
    // drop and spawn is microseconds — far shorter than before the fix.
    drop(port_guard);
    let mut child = command
        .spawn()
        .context("Bastion tunnel failed: could not start ssh")?;
    let ready = wait_for_tunnel(&mut child, local_port).await;

    ready?;
    Ok(SshTunnel { child, local_port })
}

fn reserve_local_port() -> Result<(u16, TcpListener)> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("failed to bind local tunnel port")?;
    let port = listener.local_addr()?.port();
    Ok((port, listener))
}

async fn wait_for_tunnel(child: &mut Child, local_port: u16) -> Result<()> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .context("Bastion tunnel failed: could not inspect ssh process")?
        {
            bail!("Bastion tunnel failed: ssh exited with {status}");
        }

        if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
            return Ok(());
        }

        if started.elapsed() >= Duration::from_secs(SSH_CONNECT_TIMEOUT_SECS + 4) {
            let _ = child.start_kill();
            bail!("Bastion tunnel failed: connection timeout");
        }

        sleep(Duration::from_millis(100)).await;
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
        let message = format!("ssh command failed: {}", combined.trim());
        if is_connection_reuse_error(&message) {
            invalidate_connection(server, &control_path).await;
        }
        return Err(anyhow!(message));
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
        .arg("ServerAliveInterval=10")
        .arg("-o")
        .arg("ServerAliveCountMax=6")
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

    // The output was already shown live; the final event only needs to carry
    // command failure state back to the UI.
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
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create backup directory {}", parent.display()))?;
    }

    let batch_path = TempPathGuard::new(
        std::env::temp_dir().join(format!("nodenet-sftp-{}.batch", Uuid::new_v4())),
    );
    fs::write(
        batch_path.path(),
        format!(
            "get {} {}\n",
            sftp_quote(remote_path),
            sftp_quote(&local_path.display().to_string())
        ),
    )
    .with_context(|| format!("failed to write sftp batch {}", batch_path.path().display()))?;

    let connection = get_or_create_connection(app, server).await?;
    let control_path = {
        let mut connection = connection.lock().await;
        connection.last_used = Instant::now();
        connection.control_path.clone()
    };

    let askpass = create_transfer_askpass(app, server).await?;
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
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg(format!("ControlPersist={SSH_IDLE_TIMEOUT_SECS}"))
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("PreferredAuthentications=publickey,password")
        .arg("-o")
        .arg("NumberOfPasswordPrompts=1")
        .arg("-b")
        .arg(batch_path.path());

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    apply_bastion_proxy(&mut command, server);
    apply_askpass_env(&mut command, askpass.as_ref());

    command.arg(format!("{}@{}", server.ssh_user, server.host));

    let output = match timeout(
        Duration::from_secs(SSH_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(anyhow!("failed to execute sftp: {error}")),
        Err(_) => {
            invalidate_connection(server, &control_path).await;
            bail!("sftp download timed out");
        }
    };

    if !output.status.success() {
        let message = format!(
            "sftp download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        if is_connection_reuse_error(&message) {
            invalidate_connection(server, &control_path).await;
        }
        return Err(anyhow!(message));
    }

    Ok(())
}

pub async fn upload_file(
    app: &AppHandle,
    server: &ServerConfig,
    local_path: &Path,
    remote_path: &str,
) -> Result<()> {
    retry_ssh_operation(|| async { upload_file_once(app, server, local_path, remote_path).await })
        .await
}

async fn upload_file_once(
    app: &AppHandle,
    server: &ServerConfig,
    local_path: &Path,
    remote_path: &str,
) -> Result<()> {
    if !local_path.is_file() {
        bail!("local file does not exist: {}", local_path.display());
    }

    let batch_path = TempPathGuard::new(
        std::env::temp_dir().join(format!("nodenet-sftp-{}.batch", Uuid::new_v4())),
    );
    fs::write(
        batch_path.path(),
        format!(
            "put {} {}\n",
            sftp_quote(&local_path.display().to_string()),
            sftp_quote(remote_path)
        ),
    )
    .with_context(|| format!("failed to write sftp batch {}", batch_path.path().display()))?;

    let connection = get_or_create_connection(app, server).await?;
    let control_path = {
        let mut connection = connection.lock().await;
        connection.last_used = Instant::now();
        connection.control_path.clone()
    };

    let askpass = create_transfer_askpass(app, server).await?;
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
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg(format!("ControlPersist={SSH_IDLE_TIMEOUT_SECS}"))
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("PreferredAuthentications=publickey,password")
        .arg("-o")
        .arg("NumberOfPasswordPrompts=1")
        .arg("-b")
        .arg(batch_path.path());

    if let Some(key_path) = &server.ssh_key_path {
        command
            .arg("-i")
            .arg(expand_tilde(key_path))
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }

    apply_bastion_proxy(&mut command, server);
    apply_askpass_env(&mut command, askpass.as_ref());

    command.arg(format!("{}@{}", server.ssh_user, server.host));

    let output = match timeout(
        Duration::from_secs(SSH_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(anyhow!("failed to execute sftp: {error}")),
        Err(_) => {
            invalidate_connection(server, &control_path).await;
            bail!("sftp upload timed out");
        }
    };

    if !output.status.success() {
        let message = format!(
            "sftp upload failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        if is_connection_reuse_error(&message) {
            invalidate_connection(server, &control_path).await;
        }
        return Err(anyhow!(message));
    }

    Ok(())
}

async fn get_or_create_connection(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Arc<Mutex<SshConnection>>> {
    let account = connection_pool_account(server);
    let mut pool = SSH_POOL.lock().await;
    if let Some(connection) = pool.get(&account).cloned() {
        let (is_fresh, control_path) = {
            let connection = connection.lock().await;
            (
                connection.last_used.elapsed() < Duration::from_secs(SSH_IDLE_TIMEOUT_SECS),
                connection.control_path.clone(),
            )
        };
        let alive = is_fresh && is_connection_alive(server, &control_path).await;

        if alive {
            return Ok(connection);
        }

        pool.remove(&account);
        close_master(server, &control_path).await;
    }

    let control_path = ssh_control_path()?;
    open_master(app, server, &control_path).await?;
    let connection = Arc::new(Mutex::new(SshConnection {
        control_path,
        last_used: Instant::now(),
    }));
    pool.insert(account, Arc::clone(&connection));
    Ok(connection)
}

pub async fn close_server_connections(server: &ServerConfig) {
    let account = connection_pool_account(server);
    let connection = SSH_POOL.lock().await.remove(&account);
    if let Some(connection) = connection {
        let control_path = connection.lock().await.control_path.clone();
        close_master(server, &control_path).await;
    }
}

async fn invalidate_connection(server: &ServerConfig, control_path: &Path) {
    let account = connection_pool_account(server);
    let connection = {
        let mut pool = SSH_POOL.lock().await;
        let matches = if let Some(connection) = pool.get(&account) {
            connection.lock().await.control_path == control_path
        } else {
            false
        };

        if matches {
            pool.remove(&account)
        } else {
            None
        }
    };

    if connection.is_some() {
        close_master(server, control_path).await;
    }
    let _ = fs::remove_file(control_path);
}

fn ssh_socket_dir() -> PathBuf {
    PathBuf::from("/tmp/nodenet-ssh")
}

fn ssh_control_path() -> Result<PathBuf> {
    let dir = ssh_socket_dir();
    fs::create_dir_all(&dir).context("failed to create ssh socket directory")?;
    Ok(dir.join(format!("c-{}.sock", Uuid::new_v4().simple())))
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
    let askpass =
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
        .arg("ServerAliveInterval=10")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
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

    if let Some(askpass) = &askpass {
        command
            .env("SSH_ASKPASS", askpass.path())
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

fn is_connection_reuse_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("mux_client_request_session")
        || message.contains("read from master failed")
        || message.contains("broken pipe")
        || message.contains("control socket")
        || message.contains("connection closed")
        || message.contains("connection reset")
        || message.contains("timed out")
}

fn create_askpass_script(
    target_secret: Option<&str>,
    bastion_secret: Option<&str>,
    server: &ServerConfig,
) -> Result<Option<TempPathGuard>> {
    if target_secret.is_none() && bastion_secret.is_none() {
        return Ok(None);
    }

    let path = std::env::temp_dir().join(format!("nodenet-askpass-{}.sh", Uuid::new_v4()));
    let bastion_account = bastion_keychain_account(server).unwrap_or_default();
    let bastion_host = server.bastion_host.as_deref().unwrap_or_default();
    let target_secret = target_secret.unwrap_or_default();
    let bastion_secret = bastion_secret.unwrap_or(target_secret);
    let script = format!(
        "#!/bin/sh\nprompt=\"$1\"\ncase \"$prompt\" in\n  *{}*|*{}*) printf '%s\\n' {} ;;\n  *) printf '%s\\n' {} ;;\nesac\n",
        shell_case_pattern(&bastion_account),
        shell_case_pattern(bastion_host),
        shell_single_quote(bastion_secret),
        shell_single_quote(target_secret)
    );

    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .with_context(|| format!("failed to create askpass script {}", path.display()))?;
        file.write_all(script.as_bytes())
            .with_context(|| format!("failed to write askpass script {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, script)
            .with_context(|| format!("failed to write askpass script {}", path.display()))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions)?;
    }

    Ok(Some(TempPathGuard::new(path)))
}

async fn create_transfer_askpass(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<TempPathGuard>> {
    let target_secret = if server.ssh_key_path.is_some() {
        read_key_passphrase(app, server).await?
    } else {
        read_password(app, server).await?
    };
    let bastion_secret = read_bastion_password(app, server).await?;
    create_askpass_script(target_secret.as_deref(), bastion_secret.as_deref(), server)
}

fn apply_askpass_env(command: &mut Command, askpass: Option<&TempPathGuard>) {
    if let Some(askpass) = askpass {
        command
            .env("SSH_ASKPASS", askpass.path())
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", ":0");
    }
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

pub fn has_bastion(server: &ServerConfig) -> bool {
    bastion_host(server).is_some()
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

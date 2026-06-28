use crate::{
    config::{config_dir, find_server, ServerConfig},
    ssh,
    util::expand_tilde,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use russh::client;
use russh::keys::{key::PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, OnceLock}, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

const TERMINAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static KNOWN_HOSTS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn known_hosts_lock() -> &'static Mutex<()> {
    KNOWN_HOSTS_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Default)]
pub struct TerminalState {
    sessions: Mutex<HashMap<String, mpsc::UnboundedSender<TerminalCommand>>>,
}

#[derive(Debug)]
enum TerminalCommand {
    Input(String),
    Resize { cols: u32, rows: u32 },
    Disconnect,
}

#[derive(Debug, Clone)]
struct TerminalClient {
    host_key: String,
    known_hosts_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalOutputEvent {
    session_id: String,
    server_id: String,
    data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalStatusEvent {
    session_id: String,
    server_id: String,
    status: String,
    message: String,
}

#[async_trait]
impl client::Handler for TerminalClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        verify_known_host(&self.known_hosts_path, &self.host_key, server_public_key).await?;
        Ok(true)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownHosts(HashMap<String, String>);

#[tauri::command]
pub async fn terminal_connect(
    server_id: String,
    session_id: Option<String>,
    cols: u32,
    rows: u32,
    app: AppHandle,
    state: State<'_, TerminalState>,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    let session_id = session_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let (tx, rx) = mpsc::unbounded_channel();

    {
        let mut sessions = state.sessions.lock().await;
        // If a worker already exists for this session_id, shut it down before replacing.
        if let Some(old_tx) = sessions.remove(&session_id) {
            let _ = old_tx.send(TerminalCommand::Disconnect);
        }
        sessions.insert(session_id.clone(), tx);
    }

    tauri::async_runtime::spawn(run_terminal_worker(
        app,
        session_id.clone(),
        server,
        cols.max(20),
        rows.max(5),
        rx,
    ));

    Ok(session_id)
}

#[tauri::command]
pub async fn terminal_input(
    session_id: String,
    data: String,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    send_terminal_command(&state, &session_id, TerminalCommand::Input(data)).await
}

#[tauri::command]
pub async fn terminal_resize(
    session_id: String,
    cols: u32,
    rows: u32,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    send_terminal_command(
        &state,
        &session_id,
        TerminalCommand::Resize {
            cols: cols.max(20),
            rows: rows.max(5),
        },
    )
    .await
}

#[tauri::command]
pub async fn terminal_disconnect(
    session_id: String,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    let sender = state.sessions.lock().await.remove(&session_id);
    if let Some(sender) = sender {
        let _ = sender.send(TerminalCommand::Disconnect);
    }
    Ok(())
}

async fn send_terminal_command(
    state: &State<'_, TerminalState>,
    session_id: &str,
    command: TerminalCommand,
) -> Result<(), String> {
    let sessions = state.sessions.lock().await;
    let sender = sessions
        .get(session_id)
        .ok_or_else(|| "terminal session was not found".to_string())?;
    sender.send(command).map_err(|_| {
        "terminal session is not accepting commands; it may be reconnecting".to_string()
    })
}

async fn run_terminal_worker(
    app: AppHandle,
    session_id: String,
    server: ServerConfig,
    mut cols: u32,
    mut rows: u32,
    mut rx: mpsc::UnboundedReceiver<TerminalCommand>,
) {
    let mut reconnect_delay = Duration::from_secs(1);

    loop {
        emit_status(
            &app,
            &session_id,
            &server.id,
            "connecting",
            "Opening SSH PTY",
        );

        match connect_and_open_shell(&app, &server, cols, rows).await {
            Ok(mut live) => {
                reconnect_delay = Duration::from_secs(1);
                emit_status(&app, &session_id, &server.id, "connected", "Connected");

                match run_live_terminal(&app, &session_id, &server.id, &mut live, &mut rx).await {
                    LiveResult::Disconnect | LiveResult::Done => {
                        let _ = live.channel.close().await;
                        let _ = live
                            .session
                            .disconnect(Disconnect::ByApplication, "", "en")
                            .await;
                        if let Some(bastion_session) = &mut live.bastion_session {
                            let _ = bastion_session
                                .disconnect(Disconnect::ByApplication, "", "en")
                                .await;
                        }
                        emit_status(
                            &app,
                            &session_id,
                            &server.id,
                            "disconnected",
                            "Disconnected",
                        );
                        break;
                    }
                    LiveResult::Reconnect {
                        next_cols,
                        next_rows,
                        message,
                    } => {
                        cols = next_cols;
                        rows = next_rows;
                        emit_status(&app, &session_id, &server.id, "reconnecting", &message);
                    }
                }
            }
            Err(error) => {
                emit_status(
                    &app,
                    &session_id,
                    &server.id,
                    "reconnecting",
                    &format!("SSH connection failed: {error}"),
                );
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(reconnect_delay) => {}
            Some(command) = rx.recv() => {
                match command {
                    TerminalCommand::Disconnect => {
                        emit_status(&app, &session_id, &server.id, "disconnected", "Disconnected");
                        break;
                    }
                    TerminalCommand::Resize { cols: next_cols, rows: next_rows } => {
                        cols = next_cols;
                        rows = next_rows;
                    }
                    TerminalCommand::Input(_) => {}
                }
            }
            else => break,
        }

        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(8));
    }

    // Remove the session entry when the worker exits so callers don't see a stale sender.
    app.state::<TerminalState>()
        .sessions
        .lock()
        .await
        .remove(&session_id);
}

struct LiveTerminal {
    session: client::Handle<TerminalClient>,
    bastion_session: Option<client::Handle<TerminalClient>>,
    channel: russh::Channel<client::Msg>,
    cols: u32,
    rows: u32,
}

enum LiveResult {
    Disconnect,
    Done,
    Reconnect {
        next_cols: u32,
        next_rows: u32,
        message: String,
    },
}

async fn connect_and_open_shell(
    app: &AppHandle,
    server: &ServerConfig,
    cols: u32,
    rows: u32,
) -> Result<LiveTerminal> {
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(12)),
        ..Default::default()
    });
    let (mut session, bastion_session) = connect_terminal_session(app, server, config).await?;

    tokio::time::timeout(
        TERMINAL_CONNECT_TIMEOUT,
        authenticate(app, &mut session, server),
    )
    .await
    .context("SSH terminal authentication timed out")??;

    let channel = session.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;

    Ok(LiveTerminal {
        session,
        bastion_session,
        channel,
        cols,
        rows,
    })
}

async fn connect_terminal_session(
    app: &AppHandle,
    server: &ServerConfig,
    config: Arc<client::Config>,
) -> Result<(
    client::Handle<TerminalClient>,
    Option<client::Handle<TerminalClient>>,
)> {
    let known_hosts_path = known_hosts_path()?;
    let Some(bastion_host) = server
        .bastion_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        let session = tokio::time::timeout(
            TERMINAL_CONNECT_TIMEOUT,
            client::connect(
                config,
                (server.host.as_str(), server.ssh_port),
                TerminalClient {
                    host_key: terminal_host_key(server),
                    known_hosts_path,
                },
            ),
        )
        .await
        .context("SSH terminal connection timed out")?
        .with_context(|| format!("failed to connect to {}:{}", server.host, server.ssh_port))?;
        return Ok((session, None));
    };

    let bastion_port = server.bastion_port.unwrap_or(22);
    let mut bastion_session = tokio::time::timeout(
        TERMINAL_CONNECT_TIMEOUT,
        client::connect(
            Arc::clone(&config),
            (bastion_host, bastion_port),
            TerminalClient {
                host_key: format!("{bastion_host}:{bastion_port}"),
                known_hosts_path: known_hosts_path.clone(),
            },
        ),
    )
    .await
    .context("bastion SSH terminal connection timed out")?
    .with_context(|| format!("failed to connect to bastion {bastion_host}:{bastion_port}"))?;

    tokio::time::timeout(
        TERMINAL_CONNECT_TIMEOUT,
        authenticate_bastion(app, &mut bastion_session, server),
    )
    .await
    .context("bastion SSH terminal authentication timed out")??;

    let channel = bastion_session
        .channel_open_direct_tcpip(
            server.host.clone(),
            u32::from(server.ssh_port),
            "127.0.0.1",
            0,
        )
        .await
        .with_context(|| {
            format!(
                "failed to open bastion tunnel to {}:{}",
                server.host, server.ssh_port
            )
        })?;

    let session = tokio::time::timeout(
        TERMINAL_CONNECT_TIMEOUT,
        client::connect_stream(
            config,
            channel.into_stream(),
            TerminalClient {
                host_key: terminal_host_key(server),
                known_hosts_path,
            },
        ),
    )
    .await
    .context("target SSH terminal connection through bastion timed out")?
    .with_context(|| {
        format!(
            "failed to connect to {}:{} through bastion",
            server.host, server.ssh_port
        )
    })?;

    Ok((session, Some(bastion_session)))
}

async fn authenticate(
    app: &AppHandle,
    session: &mut client::Handle<TerminalClient>,
    server: &ServerConfig,
) -> Result<()> {
    if let Some(key_path) = &server.ssh_key_path {
        let passphrase = ssh::read_key_passphrase(app, server).await?;
        let private_key = load_secret_key(expand_tilde(key_path), passphrase.as_deref())
            .with_context(|| format!("failed to load SSH key {}", key_path))?;
        let accepted = session
            .authenticate_publickey(
                server.ssh_user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(private_key), None)?,
            )
            .await?;

        if !accepted {
            bail!("SSH public key authentication failed");
        }

        return Ok(());
    }

    let password = ssh::read_password(app, server)
        .await?
        .context("SSH password is not saved in Keychain")?;
    let accepted = session
        .authenticate_password(server.ssh_user.clone(), password)
        .await?;

    if !accepted {
        bail!("SSH password authentication failed");
    }

    Ok(())
}

async fn authenticate_bastion(
    app: &AppHandle,
    session: &mut client::Handle<TerminalClient>,
    server: &ServerConfig,
) -> Result<()> {
    let username = server
        .bastion_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server.ssh_user.as_str());
    if let Some(key_path) = server
        .bastion_ssh_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let passphrase = ssh::read_bastion_password(app, server).await?;
        let private_key = load_secret_key(expand_tilde(key_path), passphrase.as_deref())
            .with_context(|| format!("failed to load bastion SSH key {}", key_path))?;
        let accepted = session
            .authenticate_publickey(
                username.to_string(),
                PrivateKeyWithHashAlg::new(Arc::new(private_key), None)?,
            )
            .await?;

        if !accepted {
            bail!("bastion public key authentication failed");
        }

        return Ok(());
    }

    let password = ssh::read_bastion_password(app, server)
        .await?
        .context("bastion password is not saved in Keychain")?;
    let accepted = session
        .authenticate_password(username.to_string(), password)
        .await?;

    if !accepted {
        bail!("bastion password authentication failed");
    }

    Ok(())
}

async fn run_live_terminal(
    app: &AppHandle,
    session_id: &str,
    server_id: &str,
    live: &mut LiveTerminal,
    rx: &mut mpsc::UnboundedReceiver<TerminalCommand>,
) -> LiveResult {
    loop {
        tokio::select! {
            Some(command) = rx.recv() => {
                match command {
                    TerminalCommand::Input(data) => {
                        if let Err(error) = live.channel.data(data.as_bytes()).await {
                            return LiveResult::Reconnect {
                                next_cols: live.cols,
                                next_rows: live.rows,
                                message: format!("SSH input failed: {error}"),
                            };
                        }
                    }
                    TerminalCommand::Resize { cols, rows } => {
                        live.cols = cols;
                        live.rows = rows;
                        if let Err(error) = live.channel.window_change(cols, rows, 0, 0).await {
                            return LiveResult::Reconnect {
                                next_cols: live.cols,
                                next_rows: live.rows,
                                message: format!("SSH resize failed: {error}"),
                            };
                        }
                    }
                    TerminalCommand::Disconnect => return LiveResult::Disconnect,
                }
            }
            message = live.channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        emit_output(app, session_id, server_id, &String::from_utf8_lossy(&data));
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        emit_output(app, session_id, server_id, &String::from_utf8_lossy(&data));
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        if exit_status == 0 {
                            return LiveResult::Done;
                        }
                        return LiveResult::Reconnect {
                            next_cols: live.cols,
                            next_rows: live.rows,
                            message: format!("Remote shell exited with status {exit_status}"),
                        };
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        return LiveResult::Reconnect {
                            next_cols: live.cols,
                            next_rows: live.rows,
                            message: "SSH channel closed".to_string(),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}

fn emit_output(app: &AppHandle, session_id: &str, server_id: &str, data: &str) {
    let _ = app.emit(
        "terminal-output",
        TerminalOutputEvent {
            session_id: session_id.to_string(),
            server_id: server_id.to_string(),
            data: data.to_string(),
        },
    );
}

fn emit_status(app: &AppHandle, session_id: &str, server_id: &str, status: &str, message: &str) {
    let _ = app.emit(
        "terminal-status",
        TerminalStatusEvent {
            session_id: session_id.to_string(),
            server_id: server_id.to_string(),
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

async fn verify_known_host(
    path: &PathBuf,
    host_key: &str,
    server_public_key: &russh::keys::ssh_key::PublicKey,
) -> std::result::Result<(), russh::Error> {
    let fingerprint = general_purpose::STANDARD.encode(
        server_public_key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .as_ref(),
    );

    let _guard = known_hosts_lock().lock().await;

    let mut known_hosts = read_known_hosts(path).await?;

    if let Some(stored_fingerprint) = known_hosts.0.get(host_key) {
        if stored_fingerprint == &fingerprint {
            return Ok(());
        }

        return Err(std::io::Error::other(format!(
            "SSH host key for '{host_key}' has changed. \
                If the server was reinstalled, remove it from Settings → Known Hosts."
        ))
        .into());
    }

    known_hosts.0.insert(host_key.to_string(), fingerprint);
    write_known_hosts(path, &known_hosts).await?;

    Ok(())
}

async fn read_known_hosts(path: &PathBuf) -> std::result::Result<KnownHosts, russh::Error> {
    if !path.exists() {
        return Ok(KnownHosts::default());
    }

    let raw = tokio::fs::read_to_string(path).await?;
    Ok(serde_json::from_str::<KnownHosts>(&raw)
        .or_else(|_| serde_json::from_str::<HashMap<String, String>>(&raw).map(KnownHosts))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?)
}

async fn write_known_hosts(
    path: &PathBuf,
    known_hosts: &KnownHosts,
) -> std::result::Result<(), russh::Error> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(
        path,
        serde_json::to_string_pretty(&known_hosts.0)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
    ).await?;

    Ok(())
}

fn known_hosts_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("known_hosts.json"))
}

fn terminal_host_key(server: &ServerConfig) -> String {
    format!("{}:{}", server.host, server.ssh_port)
}

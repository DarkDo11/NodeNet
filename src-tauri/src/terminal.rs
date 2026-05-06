use crate::{config::find_server, config::ServerConfig, ssh};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use russh::client;
use russh::keys::{key::PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use serde::Serialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

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
struct TerminalClient;

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
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(true)
    }
}

#[tauri::command]
pub async fn terminal_connect(
    server_id: String,
    cols: u32,
    rows: u32,
    app: AppHandle,
    state: State<'_, TerminalState>,
) -> Result<String, String> {
    let server = find_server(&server_id).map_err(|error| error.to_string())?;
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::unbounded_channel();

    state.sessions.lock().await.insert(session_id.clone(), tx);

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

        match connect_and_open_shell(&server, cols, rows).await {
            Ok(mut live) => {
                reconnect_delay = Duration::from_secs(1);
                emit_status(&app, &session_id, &server.id, "connected", "Connected");

                match run_live_terminal(&app, &session_id, &server.id, &mut live, &mut rx).await {
                    LiveResult::Disconnect => {
                        let _ = live.channel.close().await;
                        let _ = live
                            .session
                            .disconnect(Disconnect::ByApplication, "", "en")
                            .await;
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
}

struct LiveTerminal {
    session: client::Handle<TerminalClient>,
    channel: russh::Channel<client::Msg>,
    cols: u32,
    rows: u32,
}

enum LiveResult {
    Disconnect,
    Reconnect {
        next_cols: u32,
        next_rows: u32,
        message: String,
    },
}

async fn connect_and_open_shell(
    server: &ServerConfig,
    cols: u32,
    rows: u32,
) -> Result<LiveTerminal> {
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(12)),
        ..Default::default()
    });
    let mut session = client::connect(
        config,
        (server.host.as_str(), server.ssh_port),
        TerminalClient,
    )
    .await
    .with_context(|| format!("failed to connect to {}:{}", server.host, server.ssh_port))?;

    authenticate(&mut session, server).await?;

    let channel = session.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;

    Ok(LiveTerminal {
        session,
        channel,
        cols,
        rows,
    })
}

async fn authenticate(
    session: &mut client::Handle<TerminalClient>,
    server: &ServerConfig,
) -> Result<()> {
    if let Some(key_path) = &server.ssh_key_path {
        let private_key = load_secret_key(expand_tilde(key_path), None)
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

    let password = ssh::read_password(server)
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

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest).display().to_string();
        }
    }

    path.to_string()
}

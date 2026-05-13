use crate::{config::load_config, keychain, metrics, ssh, three_x_ui};
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, OsRng},
    Aes256Gcm, KeyInit, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_EVENTS: usize = 500;
const EVENTS_APP_DIR: &str = "NodeNet";
const EVENTS_KEYCHAIN_SERVICE: &str = "nodenet.events";
const EVENTS_KEYCHAIN_ACCOUNT: &str = "events-aes-256-gcm";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub id: String,
    pub level: String,
    pub kind: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Default)]
pub struct AlertsState {
    events: Mutex<VecDeque<AlertEvent>>,
    runtime: Mutex<AlertsRuntime>,
}

#[derive(Default)]
struct AlertsRuntime {
    down_servers: HashSet<String>,
    high_cpu_since: HashMap<String, Instant>,
    high_cpu_alerted: HashSet<String>,
    limited_clients: HashSet<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedEventsFile {
    version: u8,
    cipher: String,
    nonce: String,
    data: String,
}

#[tauri::command]
pub async fn get_events(state: State<'_, AlertsState>) -> Result<Vec<AlertEvent>, String> {
    Ok(state.events.lock().await.iter().cloned().collect())
}

pub async fn load_events_into_state(app: &AppHandle) -> Result<()> {
    let loaded = load_events(app).await?;
    let state = app.state::<AlertsState>();
    let mut events = state.events.lock().await;
    *events = loaded.into_iter().collect();
    Ok(())
}

pub fn start_alert_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = poll_once(&app).await {
                let _ = app.emit("alert-error", format!("alert poller failed: {error}"));
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn poll_once(app: &AppHandle) -> Result<()> {
    let config = load_config()?;
    let mut handles = Vec::with_capacity(config.servers.len());

    for server in config.servers {
        let app = app.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), poll_server(&app, server)).await
        }));
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                let _ = app.emit("alert-error", format!("server alert poll failed: {error}"));
            }
            Ok(Err(_elapsed)) => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

async fn poll_server(app: &AppHandle, server: crate::config::ServerConfig) -> Result<()> {
    let ping_result = ssh::ping(app, &server).await;

    if ping_result.status == "offline" {
        let should_emit = {
            let state = app.state::<AlertsState>();
            let mut runtime = state.runtime.lock().await;
            runtime.down_servers.insert(server.id.clone())
        };

        if should_emit {
            push_event(
                app,
                "error",
                "server_down",
                Some(server.id.clone()),
                Some(server.name.clone()),
                format!("{} is unavailable: {}", server.name, ping_result.message),
            )
            .await?;
        }

        return Ok(());
    } else {
        let state = app.state::<AlertsState>();
        state.runtime.lock().await.down_servers.remove(&server.id);
    }

    if let Ok(sample) = metrics::collect(app, &server).await {
        let mut event_to_emit = None;
        {
            let state = app.state::<AlertsState>();
            let mut runtime = state.runtime.lock().await;

            if sample.cpu_percent > 90.0 {
                let since = runtime
                    .high_cpu_since
                    .entry(server.id.clone())
                    .or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(60)
                    && runtime.high_cpu_alerted.insert(server.id.clone())
                {
                    event_to_emit = Some(format!(
                        "{} CPU has been above 90% for more than 60 sec ({:.1}%)",
                        server.name, sample.cpu_percent
                    ));
                }
            } else {
                runtime.high_cpu_since.remove(&server.id);
                runtime.high_cpu_alerted.remove(&server.id);
            }
        }

        if let Some(message) = event_to_emit {
            push_event(
                app,
                "warn",
                "cpu_high",
                Some(server.id.clone()),
                Some(server.name.clone()),
                message,
            )
            .await?;
        }
    }

    if server.panel_url.is_some() {
        let Ok(inbounds) = three_x_ui::get_inbounds(app, &server).await else {
            return Ok(());
        };

        for inbound in inbounds {
            let Ok(clients) = three_x_ui::get_clients(app, &server, inbound.id).await else {
                continue;
            };

            for client in clients {
                let used = client.up.saturating_add(client.down);
                let client_key = format!("{}:{}:{}", server.id, inbound.id, client.id);
                let over_limit = client.total > 0 && (used as f64 / client.total as f64) >= 0.95;
                let should_emit = {
                    let state = app.state::<AlertsState>();
                    let mut runtime = state.runtime.lock().await;
                    if over_limit {
                        runtime.limited_clients.insert(client_key.clone())
                    } else {
                        runtime.limited_clients.remove(&client_key);
                        false
                    }
                };

                if should_emit {
                    push_event(
                        app,
                        "warn",
                        "client_traffic_limit",
                        Some(server.id.clone()),
                        Some(server.name.clone()),
                        format!(
                            "{} used {:.1}% of traffic limit on {}",
                            client.email, client.used_percent, server.name
                        ),
                    )
                    .await?;
                }
            }
        }
    }

    Ok(())
}

async fn push_event(
    app: &AppHandle,
    level: &str,
    kind: &str,
    server_id: Option<String>,
    server_name: Option<String>,
    message: String,
) -> Result<()> {
    let event = AlertEvent {
        id: Uuid::new_v4().to_string(),
        level: level.to_string(),
        kind: kind.to_string(),
        server_id,
        server_name,
        message,
        timestamp: Utc::now(),
    };

    let snapshot = {
        let state = app.state::<AlertsState>();
        let mut events = state.events.lock().await;
        events.push_front(event.clone());
        while events.len() > MAX_EVENTS {
            events.pop_back();
        }
        events.iter().cloned().collect::<Vec<_>>()
    };

    save_events(app, &snapshot).await?;
    notify(app, &event);
    let _ = app.emit("alert-event", event);

    Ok(())
}

fn notify(app: &AppHandle, event: &AlertEvent) {
    let title = match event.level.as_str() {
        "error" => "NodeNet error",
        "warn" => "NodeNet warning",
        _ => "NodeNet",
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(event.message.clone())
        .show();
}

async fn load_events(app: &AppHandle) -> Result<Vec<AlertEvent>> {
    let path = events_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read events log {}", path.display()))?;

    if raw.trim_start().starts_with('[') {
        let events = serde_json::from_str::<Vec<AlertEvent>>(&raw)
            .with_context(|| format!("failed to parse legacy events log {}", path.display()))?;
        save_events(app, &events).await?;
        return Ok(events);
    }

    let encrypted = serde_json::from_str::<EncryptedEventsFile>(&raw)
        .with_context(|| format!("failed to parse encrypted events log {}", path.display()))?;
    let key = events_key(app).await?;
    let cipher = Aes256Gcm::new_from_slice(&key).context("failed to create AES-256 cipher")?;
    let nonce_bytes = general_purpose::STANDARD
        .decode(encrypted.nonce)
        .context("failed to decode events nonce")?;
    let data = general_purpose::STANDARD
        .decode(encrypted.data)
        .context("failed to decode events ciphertext")?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), data.as_ref())
        .map_err(|_| anyhow::anyhow!("failed to decrypt events log"))?;

    serde_json::from_slice::<Vec<AlertEvent>>(&plaintext)
        .with_context(|| format!("failed to parse events log {}", path.display()))
}

async fn save_events(app: &AppHandle, events: &[AlertEvent]) -> Result<()> {
    let path = events_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create events directory {}", parent.display()))?;
    }

    let key = events_key(app).await?;
    let cipher = Aes256Gcm::new_from_slice(&key).context("failed to create AES-256 cipher")?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let plaintext = serde_json::to_vec(events).context("failed to serialize events log")?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| anyhow::anyhow!("failed to encrypt events log"))?;
    let encrypted = EncryptedEventsFile {
        version: 1,
        cipher: "AES-256-GCM".to_string(),
        nonce: general_purpose::STANDARD.encode(nonce),
        data: general_purpose::STANDARD.encode(ciphertext),
    };

    fs::write(&path, serde_json::to_string_pretty(&encrypted)?)
        .with_context(|| format!("failed to write events log {}", path.display()))
}

async fn events_key(app: &AppHandle) -> Result<[u8; 32]> {
    if let Some(raw) =
        keychain::read_password(app, EVENTS_KEYCHAIN_SERVICE, EVENTS_KEYCHAIN_ACCOUNT).await?
    {
        let decoded = general_purpose::STANDARD
            .decode(raw)
            .context("failed to decode events encryption key")?;
        if decoded.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            return Ok(key);
        }
    }

    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    keychain::save_password(
        app,
        EVENTS_KEYCHAIN_SERVICE,
        EVENTS_KEYCHAIN_ACCOUNT,
        &general_purpose::STANDARD.encode(key),
    )
    .await?;
    Ok(key)
}

fn events_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("unable to resolve user directories")?;
    Ok(base_dirs
        .data_dir()
        .join(EVENTS_APP_DIR)
        .join("events.json"))
}

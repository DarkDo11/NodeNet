use crate::{config::load_config, metrics, ssh, three_x_ui};
use anyhow::{Context, Result};
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
const EVENTS_APP_DIR: &str = "vpnctrl";

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

impl AlertsState {
    pub fn load() -> Self {
        let events = load_events().unwrap_or_default().into_iter().collect();
        Self {
            events: Mutex::new(events),
            runtime: Mutex::new(AlertsRuntime::default()),
        }
    }
}

#[tauri::command]
pub async fn get_events(state: State<'_, AlertsState>) -> Result<Vec<AlertEvent>, String> {
    Ok(state.events.lock().await.iter().cloned().collect())
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

    for server in config.servers {
        let ping_result = ssh::ping(&server).await;

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

            continue;
        } else {
            let state = app.state::<AlertsState>();
            state.runtime.lock().await.down_servers.remove(&server.id);
        }

        if let Ok(sample) = metrics::collect(&server).await {
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
            let Ok(inbounds) = three_x_ui::get_inbounds(&server).await else {
                continue;
            };

            for inbound in inbounds {
                let Ok(clients) = three_x_ui::get_clients(&server, inbound.id).await else {
                    continue;
                };

                for client in clients {
                    let used = client.up.saturating_add(client.down);
                    let client_key = format!("{}:{}:{}", server.id, inbound.id, client.id);
                    let over_limit =
                        client.total > 0 && (used as f64 / client.total as f64) >= 0.95;
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

    save_events(&snapshot)?;
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

fn load_events() -> Result<Vec<AlertEvent>> {
    let path = events_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read events log {}", path.display()))?;
    serde_json::from_str::<Vec<AlertEvent>>(&raw)
        .with_context(|| format!("failed to parse events log {}", path.display()))
}

fn save_events(events: &[AlertEvent]) -> Result<()> {
    let path = events_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create events directory {}", parent.display()))?;
    }

    fs::write(&path, serde_json::to_string_pretty(events)?)
        .with_context(|| format!("failed to write events log {}", path.display()))
}

fn events_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("unable to resolve user directories")?;
    Ok(base_dirs
        .data_dir()
        .join(EVENTS_APP_DIR)
        .join("events.json"))
}

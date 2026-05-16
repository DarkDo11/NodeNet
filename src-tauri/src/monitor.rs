use crate::{
    config::{load_config, AppConfig, BastionConfig, ServerConfig},
    metrics::ServerMetrics,
    ssh,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
use tauri::AppHandle;
use uuid::Uuid;

const MONITOR_DATA_DIR: &str = "/var/lib/nodenet-monitor";
const MONITOR_CONFIG_DIR: &str = "/etc/nodenet-monitor";
const MONITOR_CONFIG_PATH: &str = "/etc/nodenet-monitor/config.json";
const MONITOR_KEYS_DIR: &str = "/var/lib/nodenet-monitor/keys";
const MONITOR_AGENT_PATH: &str = "/usr/local/bin/nodenet-monitor-agent";
const MONITOR_SERVICE_PATH: &str = "/etc/systemd/system/nodenet-monitor.service";
const MONITOR_TIMER_PATH: &str = "/etc/systemd/system/nodenet-monitor.timer";
const REMOTE_METRICS_PATH: &str = "/var/lib/nodenet-monitor/metrics-cache.json";
const REMOTE_EVENTS_PATH: &str = "/var/lib/nodenet-monitor/events.json";
const MIN_MONITOR_SAMPLE_AGE_SECS: u64 = 60;
const MAX_MONITOR_SAMPLE_AGE_SECS: u64 = 900;
const MONITOR_SAMPLE_INTERVALS_BEFORE_STALE: u64 = 4;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorAgentConfig<'a> {
    poll_interval_sec: u64,
    monitor_server_id: &'a str,
    monitor_host: &'a str,
    monitor_port: u16,
    monitor_user: &'a str,
    servers: &'a [ServerConfig],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSavedServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub country: String,
    pub panel_url: Option<String>,
    pub ssh_key_path: Option<String>,
    pub has_local_config: bool,
}

pub fn is_enabled(config: &AppConfig) -> bool {
    config
        .monitor_server_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || config
            .monitor_bastion_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub fn monitor_server(config: &AppConfig) -> Result<Option<ServerConfig>> {
    if let Some(server_id) = config
        .monitor_server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let server = config
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
            .with_context(|| format!("monitor server '{server_id}' was not found"))?;
        return Ok(Some(server));
    }

    if let Some(bastion_id) = config
        .monitor_bastion_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let bastion = config
            .bastions
            .iter()
            .find(|bastion| bastion.id == bastion_id)
            .with_context(|| format!("monitor bastion '{bastion_id}' was not found"))?;
        return Ok(Some(server_from_bastion(bastion)));
    }

    Ok(None)
}

pub async fn load_metrics_cache(app: &AppHandle) -> Result<Option<Value>> {
    let config = load_config()?;
    let Some(monitor) = monitor_server(&config)? else {
        return Ok(None);
    };

    let raw = read_remote_file(app, &monitor, REMOTE_METRICS_PATH, "{}").await?;
    let value = parse_json_value_from_output(&raw, '{', '}')
        .unwrap_or_else(|| Value::Object(Default::default()));
    Ok(Some(value))
}

pub async fn latest_metrics(app: &AppHandle, server_id: &str) -> Result<Option<ServerMetrics>> {
    let config = load_config()?;
    let max_age = monitor_sample_max_age(&config);
    let Some(cache) = load_metrics_cache(app).await? else {
        return Ok(None);
    };

    latest_metrics_from_cache(&cache, server_id, max_age).map(Some)
}

pub async fn ping_from_monitor(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<ssh::PingResult>> {
    let config = load_config()?;
    let max_age = monitor_sample_max_age(&config);
    let Some(cache) = load_metrics_cache(app).await? else {
        return Ok(None);
    };

    let Some(history) = cache.get(&server.id).and_then(Value::as_array) else {
        return Ok(Some(ssh::PingResult {
            server_id: server.id.clone(),
            latency_ms: None,
            status: "unknown".to_string(),
            message: "No monitor sample yet".to_string(),
            checked_at: Utc::now(),
        }));
    };
    let Some(latest) = stable_latest_metric(history, max_age) else {
        return Ok(Some(ssh::PingResult {
            server_id: server.id.clone(),
            latency_ms: None,
            status: "unknown".to_string(),
            message: "Monitor sample is stale".to_string(),
            checked_at: Utc::now(),
        }));
    };

    let latest_sample = history.last().unwrap_or(latest);
    let is_transient_failure = metric_is_fresh(latest_sample, Utc::now(), max_age)
        && !metric_is_online(latest_sample)
        && metric_is_online(latest);

    let is_online = latest
        .get("isOnline")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let ping_ms = latest
        .get("pingMs")
        .and_then(Value::as_f64)
        .map(|value| value.round().max(0.0) as u128);
    let status = if !is_online {
        "offline"
    } else if is_transient_failure || ping_ms.is_some_and(|value| value > 1_000) {
        "warning"
    } else {
        "online"
    };

    Ok(Some(ssh::PingResult {
        server_id: server.id.clone(),
        latency_ms: ping_ms,
        status: status.to_string(),
        message: if is_transient_failure {
            "Monitor sample, last check failed once".to_string()
        } else {
            "Monitor sample".to_string()
        },
        checked_at: Utc::now(),
    }))
}

pub async fn load_events(app: &AppHandle) -> Result<Option<Vec<crate::alerts::AlertEvent>>> {
    let config = load_config()?;
    let Some(monitor) = monitor_server(&config)? else {
        return Ok(None);
    };

    let raw = read_remote_file(app, &monitor, REMOTE_EVENTS_PATH, "[]").await?;
    let events = parse_monitor_events(&raw).context("failed to parse monitor events")?;
    Ok(Some(events))
}

pub async fn list_saved_servers(app: &AppHandle) -> Result<Vec<MonitorSavedServer>> {
    let config = load_config()?;
    let monitor = monitor_server(&config)?.context("Choose a monitor server first")?;
    let value = remote_monitor_config(app, &monitor).await?;
    let servers = value
        .get("servers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| monitor_saved_server_from_value(value, &config))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(servers)
}

pub async fn delete_saved_server(app: &AppHandle, server_id: &str) -> Result<String> {
    let config = load_config()?;
    let monitor = monitor_server(&config)?.context("Choose a monitor server first")?;
    let server_id_json = serde_json::to_string(server_id)?;
    let script = r#"
import json
import os
import pathlib

SERVER_ID = __SERVER_ID__
CONFIG_PATH = pathlib.Path("/etc/nodenet-monitor/config.json")
DATA_DIR = pathlib.Path("/var/lib/nodenet-monitor")
METRICS_PATH = DATA_DIR / "metrics-cache.json"
EVENTS_PATH = DATA_DIR / "events.json"
RUNTIME_PATH = DATA_DIR / "runtime.json"
KEYS_DIR = DATA_DIR / "keys"

def load_json(path, fallback):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception:
        return fallback

def save_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = pathlib.Path(str(path) + ".tmp")
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
    os.replace(tmp, path)

def sanitize(value):
    cleaned = "".join(character if (character.isascii() and character.isalnum()) or character in "._-" else "_" for character in value)
    cleaned = cleaned.strip("._-")
    return cleaned or "server"

config = load_json(CONFIG_PATH, {"servers": []})
servers = config.get("servers") if isinstance(config.get("servers"), list) else []
before = len(servers)
config["servers"] = [server for server in servers if str(server.get("id") or "") != SERVER_ID]
removed = before - len(config["servers"])
save_json(CONFIG_PATH, config)

metrics = load_json(METRICS_PATH, {})
if isinstance(metrics, dict):
    metrics.pop(SERVER_ID, None)
    save_json(METRICS_PATH, metrics)

events = load_json(EVENTS_PATH, [])
if isinstance(events, list):
    events = [event for event in events if str(event.get("serverId") or "") != SERVER_ID]
    save_json(EVENTS_PATH, events)

runtime = load_json(RUNTIME_PATH, {})
if isinstance(runtime, dict):
    down = runtime.get("downServers")
    if isinstance(down, list):
        runtime["downServers"] = [item for item in down if str(item) != SERVER_ID]
    high_since = runtime.get("highCpuSince")
    if isinstance(high_since, dict):
        high_since.pop(SERVER_ID, None)
    high_alerted = runtime.get("highCpuAlerted")
    if isinstance(high_alerted, list):
        runtime["highCpuAlerted"] = [item for item in high_alerted if str(item) != SERVER_ID]
    fail_counts = runtime.get("failCounts")
    if isinstance(fail_counts, dict):
        fail_counts.pop(SERVER_ID, None)
    save_json(RUNTIME_PATH, runtime)

safe_id = sanitize(SERVER_ID)
for suffix in ("target", "bastion"):
    key_path = KEYS_DIR / f"{safe_id}-{suffix}"
    try:
        key_path.unlink()
    except FileNotFoundError:
        pass

print(f"Removed {removed} monitor server record(s) for {SERVER_ID}")
"#
    .replace("__SERVER_ID__", &server_id_json);
    let command = format!(
        r#"set -e
SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi
$SUDO python3 - <<'PY'
{script}
PY
$SUDO systemctl start nodenet-monitor.service >/dev/null 2>&1 || true
"#
    );

    ssh::execute_combined(app, &monitor, &command, 60).await
}

pub async fn install_agent(app: &AppHandle) -> Result<String> {
    let config = load_config()?;
    install_agent_with_servers(app, config.servers.clone()).await
}

pub async fn reinstall_agent(app: &AppHandle) -> Result<String> {
    let config = load_config()?;
    let monitor = monitor_server(&config)?.context("Choose a monitor server first")?;
    let monitor_server_ids = remote_monitor_config(app, &monitor)
        .await?
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| json_string(value, "id"))
        .collect::<HashSet<_>>();
    let servers = if monitor_server_ids.is_empty() {
        config.servers.clone()
    } else {
        config
            .servers
            .iter()
            .filter(|server| monitor_server_ids.contains(&server.id))
            .cloned()
            .collect()
    };

    install_agent_with_servers(app, servers).await
}

async fn install_agent_with_servers(app: &AppHandle, servers: Vec<ServerConfig>) -> Result<String> {
    let config = load_config()?;
    let monitor = monitor_server(&config)?.context("Choose a monitor server first")?;
    let upload_id = Uuid::new_v4().simple().to_string();
    let agent_servers = prepare_agent_servers(app, &monitor, &servers, &upload_id).await?;
    let agent_config = MonitorAgentConfig {
        poll_interval_sec: config.poll_interval_sec.max(5),
        monitor_server_id: &monitor.id,
        monitor_host: &monitor.host,
        monitor_port: monitor.ssh_port,
        monitor_user: &monitor.ssh_user,
        servers: &agent_servers,
    };
    let agent_config = serde_json::to_string_pretty(&agent_config)?;

    let temp_dir = std::env::temp_dir().join(format!("nodenet-monitor-{upload_id}"));
    fs::create_dir_all(&temp_dir)?;

    let agent_path = temp_dir.join("nodenet-monitor-agent.py");
    let config_path = temp_dir.join("config.json");
    let service_path = temp_dir.join("nodenet-monitor.service");
    let timer_path = temp_dir.join("nodenet-monitor.timer");
    fs::write(&agent_path, agent_script())?;
    fs::write(&config_path, agent_config)?;
    fs::write(&service_path, service_unit())?;
    fs::write(&timer_path, timer_unit(config.poll_interval_sec.max(5)))?;

    let remote_agent_tmp = format!("/tmp/nodenet-monitor-agent-{upload_id}.py");
    let remote_config_tmp = format!("/tmp/nodenet-monitor-config-{upload_id}.json");
    let remote_service_tmp = format!("/tmp/nodenet-monitor-service-{upload_id}.service");
    let remote_timer_tmp = format!("/tmp/nodenet-monitor-timer-{upload_id}.timer");

    ssh::upload_file(app, &monitor, &agent_path, &remote_agent_tmp).await?;
    ssh::upload_file(app, &monitor, &config_path, &remote_config_tmp).await?;
    ssh::upload_file(app, &monitor, &service_path, &remote_service_tmp).await?;
    ssh::upload_file(app, &monitor, &timer_path, &remote_timer_tmp).await?;

    let install_command = format!(
        r#"set -e
SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi
$SUDO mkdir -p {config_dir} {data_dir}
$SUDO mkdir -p {keys_dir}
$SUDO mv {agent_tmp} {agent_path}
$SUDO mv {config_tmp} {config_path}
$SUDO mv {service_tmp} {service_path}
$SUDO mv {timer_tmp} {timer_path}
$SUDO chmod 755 {agent_path}
$SUDO chmod 644 {config_path} {service_path} {timer_path}
for key_tmp in /tmp/nodenet-monitor-key-*.{upload_id}.tmp; do
  [ -e "$key_tmp" ] || continue
  key_name=$(basename "$key_tmp")
  key_name=${{key_name#nodenet-monitor-key-}}
  key_name=${{key_name%.{upload_id}.tmp}}
  $SUDO mv "$key_tmp" "{keys_dir}/$key_name"
  $SUDO chmod 600 "{keys_dir}/$key_name"
done
$SUDO systemctl daemon-reload
$SUDO systemctl enable --now nodenet-monitor.timer
$SUDO systemctl start nodenet-monitor.service
$SUDO systemctl status nodenet-monitor.service --no-pager --lines=0 || true
"#,
        config_dir = MONITOR_CONFIG_DIR,
        data_dir = MONITOR_DATA_DIR,
        keys_dir = MONITOR_KEYS_DIR,
        upload_id = upload_id,
        agent_tmp = shell_single_quote(&remote_agent_tmp),
        agent_path = MONITOR_AGENT_PATH,
        config_tmp = shell_single_quote(&remote_config_tmp),
        config_path = MONITOR_CONFIG_PATH,
        service_tmp = shell_single_quote(&remote_service_tmp),
        service_path = MONITOR_SERVICE_PATH,
        timer_tmp = shell_single_quote(&remote_timer_tmp),
        timer_path = MONITOR_TIMER_PATH,
    );

    let output = ssh::execute_combined(app, &monitor, &install_command, 120).await?;
    let _ = fs::remove_dir_all(temp_dir);
    Ok(output)
}

fn monitor_saved_server_from_value(
    value: &Value,
    config: &AppConfig,
) -> Option<MonitorSavedServer> {
    let id = json_string(value, "id")?;
    let name = json_string(value, "name").unwrap_or_else(|| id.clone());
    let host = json_string(value, "host").unwrap_or_default();
    let ssh_user = json_string(value, "sshUser").unwrap_or_else(|| "root".to_string());
    let country = json_string(value, "country").unwrap_or_else(|| "US".to_string());
    let ssh_port = value
        .get("sshPort")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(22);
    let has_local_config = config.servers.iter().any(|server| server.id == id);

    Some(MonitorSavedServer {
        id,
        name,
        host,
        ssh_port,
        ssh_user,
        country,
        panel_url: json_string(value, "panelUrl"),
        ssh_key_path: json_string(value, "sshKeyPath"),
        has_local_config,
    })
}

fn parse_monitor_events(raw: &str) -> Result<Vec<crate::alerts::AlertEvent>> {
    let Some(value) = parse_json_value_from_output(raw, '[', ']') else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };

    Ok(items
        .iter()
        .filter_map(monitor_event_from_value)
        .collect::<Vec<_>>())
}

fn monitor_event_from_value(value: &Value) -> Option<crate::alerts::AlertEvent> {
    let timestamp = json_string(value, "timestamp")
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let message = json_string(value, "message")?;
    Some(crate::alerts::AlertEvent {
        id: json_string(value, "id").unwrap_or_else(|| Uuid::new_v4().to_string()),
        level: json_string(value, "level").unwrap_or_else(|| "info".to_string()),
        kind: json_string(value, "kind").unwrap_or_else(|| "monitor".to_string()),
        server_id: json_string(value, "serverId"),
        server_name: json_string(value, "serverName"),
        message,
        timestamp,
    })
}

fn parse_json_value_from_output(raw: &str, open: char, close: char) -> Option<Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }

    let start = trimmed.find(open)?;
    let end = trimmed.rfind(close)?;
    if end <= start {
        return None;
    }

    serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn server_from_bastion(bastion: &BastionConfig) -> ServerConfig {
    ServerConfig {
        id: format!("bastion:{}", bastion.id),
        name: bastion.name.clone(),
        host: bastion.host.clone(),
        ssh_port: bastion.port,
        ssh_user: bastion.user.clone(),
        country: "US".to_string(),
        panel_url: None,
        panel_user: None,
        ssh_key_path: bastion.ssh_key_path.clone(),
        bastion_host: None,
        bastion_port: None,
        bastion_user: None,
        bastion_ssh_key_path: None,
        ssh_key_passphrase: None,
        ssl_verify: false,
    }
}

pub async fn sync_server_ssh_key(app: &AppHandle, server_id: &str) -> Result<String> {
    let config = load_config()?;
    if !is_enabled(&config) {
        return Ok("Monitor server is not configured".to_string());
    }

    let selected = config
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .cloned()
        .with_context(|| format!("server '{server_id}' was not found"))?;
    let monitor = monitor_server(&config)?.context("Choose a monitor server first")?;
    let remote_config = remote_monitor_config(app, &monitor).await?;
    let mut monitor_server_ids = remote_config
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| json_string(value, "id"))
        .collect::<HashSet<_>>();
    monitor_server_ids.insert(selected.id.clone());

    let mut servers = config
        .servers
        .iter()
        .filter(|server| monitor_server_ids.contains(&server.id))
        .cloned()
        .collect::<Vec<_>>();
    if !servers.iter().any(|server| server.id == selected.id) {
        servers.push(selected);
    }

    install_agent_with_servers(app, servers).await
}

async fn remote_monitor_config(app: &AppHandle, monitor: &ServerConfig) -> Result<Value> {
    let raw = read_remote_file(app, monitor, MONITOR_CONFIG_PATH, r#"{"servers":[]}"#).await?;
    Ok(parse_json_value_from_output(&raw, '{', '}')
        .unwrap_or_else(|| serde_json::json!({ "servers": [] })))
}

fn latest_metrics_from_cache(
    cache: &Value,
    server_id: &str,
    max_age: ChronoDuration,
) -> Result<ServerMetrics> {
    let history = cache
        .get(server_id)
        .and_then(Value::as_array)
        .with_context(|| format!("monitor has no metrics for '{server_id}' yet"))?;
    let latest = stable_latest_metric(history, max_age)
        .with_context(|| format!("monitor has no metrics for '{server_id}' yet"))?;

    serde_json::from_value::<ServerMetrics>(latest.clone())
        .context("failed to decode latest monitor metrics")
}

fn stable_latest_metric(history: &[Value], max_age: ChronoDuration) -> Option<&Value> {
    let now = Utc::now();
    let latest_index = history
        .iter()
        .rposition(|value| metric_is_fresh(value, now, max_age))?;
    let latest = history.get(latest_index)?;
    if metric_is_online(latest) {
        return Some(latest);
    }

    if let Some(previous) = latest_index
        .checked_sub(1)
        .and_then(|index| history.get(index))
    {
        if metric_is_online(previous) && metric_is_fresh(previous, now, max_age) {
            return Some(previous);
        }
    }

    Some(latest)
}

fn monitor_sample_max_age(config: &AppConfig) -> ChronoDuration {
    let seconds = config
        .poll_interval_sec
        .max(5)
        .saturating_mul(MONITOR_SAMPLE_INTERVALS_BEFORE_STALE)
        .clamp(MIN_MONITOR_SAMPLE_AGE_SECS, MAX_MONITOR_SAMPLE_AGE_SECS);
    ChronoDuration::seconds(seconds as i64)
}

fn metric_is_fresh(value: &Value, now: DateTime<Utc>, max_age: ChronoDuration) -> bool {
    let Some(timestamp) = metric_timestamp(value) else {
        return false;
    };
    let age = now.signed_duration_since(timestamp);
    age <= max_age && age >= -ChronoDuration::minutes(5)
}

fn metric_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    json_string(value, "timestamp")
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn metric_is_online(value: &Value) -> bool {
    value
        .get("isOnline")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

async fn prepare_agent_servers(
    app: &AppHandle,
    monitor: &ServerConfig,
    servers: &[ServerConfig],
    upload_id: &str,
) -> Result<Vec<ServerConfig>> {
    let mut prepared = Vec::with_capacity(servers.len());

    for server in servers {
        let mut server = server.clone();
        if !is_same_endpoint(&server, monitor) {
            if let Some(remote_path) = upload_key_to_monitor(
                app,
                monitor,
                server.ssh_key_path.as_deref(),
                &server.id,
                "target",
                upload_id,
            )
            .await?
            {
                server.ssh_key_path = Some(remote_path);
            }
        }

        if !bastion_is_monitor(&server, monitor) {
            if let Some(remote_path) = upload_key_to_monitor(
                app,
                monitor,
                server.bastion_ssh_key_path.as_deref(),
                &server.id,
                "bastion",
                upload_id,
            )
            .await?
            {
                server.bastion_ssh_key_path = Some(remote_path);
            }
        }

        prepared.push(server);
    }

    Ok(prepared)
}

async fn upload_key_to_monitor(
    app: &AppHandle,
    monitor: &ServerConfig,
    local_path: Option<&str>,
    server_id: &str,
    suffix: &str,
    upload_id: &str,
) -> Result<Option<String>> {
    let Some(local_path) = local_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if local_path.starts_with(MONITOR_KEYS_DIR) {
        return Ok(Some(local_path.to_string()));
    }

    let local_path = crate::util::expand_tilde(local_path);
    let local_path = PathBuf::from(local_path);
    if !local_path.is_file() {
        return Ok(None);
    }

    let safe_server_id = sanitize_filename(server_id);
    let safe_suffix = sanitize_filename(suffix);
    let key_name = format!("{safe_server_id}-{safe_suffix}");
    let remote_path = format!("{MONITOR_KEYS_DIR}/{key_name}");
    let remote_tmp_path = format!("/tmp/nodenet-monitor-key-{key_name}.{upload_id}.tmp");
    ssh::upload_file(app, monitor, &local_path, &remote_tmp_path).await?;
    Ok(Some(remote_path))
}

fn is_same_endpoint(left: &ServerConfig, right: &ServerConfig) -> bool {
    left.host.trim().eq_ignore_ascii_case(right.host.trim())
        && left.ssh_port == right.ssh_port
        && left.ssh_user.trim() == right.ssh_user.trim()
}

fn bastion_is_monitor(server: &ServerConfig, monitor: &ServerConfig) -> bool {
    let Some(host) = server
        .bastion_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let user = server
        .bastion_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server.ssh_user.as_str());

    host.eq_ignore_ascii_case(monitor.host.trim())
        && server.bastion_port.unwrap_or(22) == monitor.ssh_port
        && user == monitor.ssh_user.trim()
}

fn sanitize_filename(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '_', '-'])
        .to_string();

    if sanitized.is_empty() {
        Uuid::new_v4().simple().to_string()
    } else {
        sanitized
    }
}

async fn read_remote_file(
    app: &AppHandle,
    monitor: &ServerConfig,
    remote_path: &str,
    fallback: &str,
) -> Result<String> {
    let command = format!(
        "if [ -f {path} ]; then cat {path}; else printf %s {fallback}; fi",
        path = shell_single_quote(remote_path),
        fallback = shell_single_quote(fallback),
    );
    ssh::execute(app, monitor, &command).await
}

fn service_unit() -> &'static str {
    r#"[Unit]
Description=NodeNet remote monitor agent
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/nodenet-monitor-agent
"#
}

fn timer_unit(interval_sec: u64) -> String {
    format!(
        r#"[Unit]
Description=Run NodeNet remote monitor agent

[Timer]
OnBootSec=20
OnUnitActiveSec={interval_sec}
AccuracySec=5
Persistent=true

[Install]
WantedBy=timers.target
"#
    )
}

fn agent_script() -> &'static str {
    r#"#!/usr/bin/env python3
import json
import os
import pathlib
import shlex
import subprocess
import time
from datetime import datetime, timezone

CONFIG_PATH = "/etc/nodenet-monitor/config.json"
DATA_DIR = pathlib.Path("/var/lib/nodenet-monitor")
METRICS_PATH = DATA_DIR / "metrics-cache.json"
EVENTS_PATH = DATA_DIR / "events.json"
RUNTIME_PATH = DATA_DIR / "runtime.json"
MAX_EVENTS = 500
MAX_POINTS_PER_SERVER = 50000

METRICS_SCRIPT = r'''
export LC_ALL=C
export LANG=C
export LANGUAGE=C
export LC_NUMERIC=C
RAM_TOTAL=0
RAM_USED=0
if command -v free >/dev/null 2>&1; then
read RAM_TOTAL RAM_USED <<EOF
$(free -m | awk '/Mem:/ {print $2 + 0, $3 + 0; found = 1} END {if (!found) print "0 0"}')
EOF
elif [ -r /proc/meminfo ]; then
read RAM_TOTAL RAM_USED <<EOF
$(awk '
  /^MemTotal:/ { total = int($2 / 1024) }
  /^MemAvailable:/ { available = int($2 / 1024) }
  END {
    if (total < 0) total = 0;
    if (available < 0) available = 0;
    used = total - available;
    if (used < 0) used = 0;
    print total, used;
  }
' /proc/meminfo)
EOF
fi
DISK_TOTAL=--
DISK_USED=--
DISK_PERCENT=0
if command -v df >/dev/null 2>&1; then
read DISK_TOTAL DISK_USED DISK_PERCENT <<EOF
$(df -P -h / 2>/dev/null | awk 'NR==2 {gsub("%", "", $5); print $2, $3, $5 + 0; found = 1} END {if (!found) print "-- -- 0"}')
EOF
fi
LOAD_AVERAGE="0 0 0"
if [ -r /proc/loadavg ]; then
  LOAD_AVERAGE=$(awk '{print $1, $2, $3}' /proc/loadavg)
fi
LOAD1=$(printf '%s\n' "$LOAD_AVERAGE" | awk '{print $1}')
CPU_CORES=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || awk '/^processor[[:space:]]*:/ {count++} END {print count + 0}' /proc/cpuinfo 2>/dev/null)
CPU_CORES=$(printf '%s\n' "$CPU_CORES" | awk 'NR==1 && $1 ~ /^[0-9]+$/ && $1 > 0 {print $1}')
if [ -z "$CPU_CORES" ]; then CPU_CORES=1; fi
CPU=$(awk -v load_value="$LOAD1" -v cores="$CPU_CORES" 'BEGIN { if (cores <= 0 || load_value < 0) exit 1; printf "%.1f", (load_value / cores) * 100; }')
if ! printf '%s\n' "$CPU" | awk '/^[0-9]+([.][0-9]+)?$/ { ok = 1 } END { exit ok ? 0 : 1 }'; then CPU=; fi
UPTIME_SEC=0
if [ -r /proc/uptime ]; then
  UPTIME_SEC=$(awk '{printf "%.0f", $1}' /proc/uptime)
fi
RX_BYTES=0
TX_BYTES=0
if [ -r /proc/net/dev ]; then
read RX_BYTES TX_BYTES <<EOF
$(awk 'NR>2 {
  iface = $1; gsub(":", "", iface);
  if (iface == "lo") next;
  fallback_rx += $2; fallback_tx += $10;
  if (iface ~ /^(eth|ens|enp|eno|em|p[0-9]|en[0-9]|wl|wlan|wwan|venet|bond|team|tun|tap|wg)/) {
    rx += $2; tx += $10; matched += 1;
  }
} END { if (matched > 0) printf "%.0f %.0f\n", rx, tx; else printf "%.0f %.0f\n", fallback_rx, fallback_tx; }' /proc/net/dev)
EOF
fi
printf 'cpu_percent=%s\n' "$CPU"
printf 'cpu_cores=%s\n' "$CPU_CORES"
printf 'ram_total_mb=%s\n' "$RAM_TOTAL"
printf 'ram_used_mb=%s\n' "$RAM_USED"
printf 'disk_total=%s\n' "$DISK_TOTAL"
printf 'disk_used=%s\n' "$DISK_USED"
printf 'disk_percent=%s\n' "$DISK_PERCENT"
printf 'load_average=%s\n' "$LOAD_AVERAGE"
printf 'uptime_sec=%s\n' "$UPTIME_SEC"
printf 'rx_bytes=%s\n' "$RX_BYTES"
printf 'tx_bytes=%s\n' "$TX_BYTES"
printf 'total_rx_bytes=%s\n' "$RX_BYTES"
printf 'total_tx_bytes=%s\n' "$TX_BYTES"
GOOGLE_204_MS=
if command -v curl >/dev/null 2>&1; then
  GOOGLE_204_MS=$(curl -o /dev/null -s -w '%{time_total}' --max-time 8 https://www.google.com/generate_204 2>/dev/null | awk '{ if ($1 ~ /^[0-9]+([.][0-9]+)?$/) printf "%.1f", $1 * 1000 }')
elif command -v wget >/dev/null 2>&1; then
  GOOGLE_204_MS=$({ start=$(date +%s%3N 2>/dev/null || date +%s000); wget -q -T 8 -O /dev/null https://www.google.com/generate_204 >/dev/null 2>&1 && end=$(date +%s%3N 2>/dev/null || date +%s000) && awk -v start="$start" -v end="$end" 'BEGIN { printf "%.1f", end - start }'; } || true)
fi
printf 'google_204_ms=%s\n' "$GOOGLE_204_MS"
'''

def utc_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")

def load_json(path, fallback):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception:
        return fallback

def save_json(path, value):
    tmp = pathlib.Path(str(path) + ".tmp")
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
    os.replace(tmp, path)

def parse_float(value, default=0.0):
    try:
        return float(str(value).strip().replace(",", "."))
    except Exception:
        return default

def parse_int(value, default=0):
    try:
        return int(float(str(value).strip()))
    except Exception:
        return default

def round_one(value):
    return round(float(value), 1)

def format_uptime(seconds):
    seconds = int(seconds)
    days = seconds // 86400
    hours = (seconds % 86400) // 3600
    minutes = (seconds % 3600) // 60
    if days > 0:
        return f"{days}d {hours}h"
    if hours > 0:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"

def parse_metrics(server_id, output, ping_ms):
    values = {}
    for line in output.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    load = [parse_float(item) for item in values.get("load_average", "0 0 0").split()[:3]]
    while len(load) < 3:
        load.append(0.0)
    cores = max(1, parse_int(values.get("cpu_cores"), 1))
    cpu = parse_float(values.get("cpu_percent"), (load[0] / cores) * 100.0)
    ram_total = parse_int(values.get("ram_total_mb"))
    ram_used = parse_int(values.get("ram_used_mb"))
    ram_percent = (ram_used / ram_total * 100.0) if ram_total else 0.0
    rx = parse_int(values.get("rx_bytes"))
    tx = parse_int(values.get("tx_bytes"))
    total_rx = parse_int(values.get("total_rx_bytes"), rx)
    total_tx = parse_int(values.get("total_tx_bytes"), tx)
    uptime_sec = parse_int(values.get("uptime_sec"))
    return {
        "serverId": server_id,
        "timestamp": utc_now(),
        "cpuPercent": round_one(max(0.0, cpu)),
        "ramUsedMb": ram_used,
        "ramTotalMb": ram_total,
        "ramPercent": round_one(max(0.0, min(100.0, ram_percent))),
        "diskUsed": values.get("disk_used") or "--",
        "diskTotal": values.get("disk_total") or "--",
        "diskPercent": round_one(max(0.0, min(100.0, parse_float(values.get("disk_percent"))))),
        "loadAverage": [round_one(item) for item in load[:3]],
        "uptimeSec": uptime_sec,
        "uptime": format_uptime(uptime_sec),
        "rxBytes": rx,
        "txBytes": tx,
        "totalRxBytes": total_rx,
        "totalTxBytes": total_tx,
        "totalTrafficBytes": total_rx + total_tx,
        "pingMs": round_one(parse_float(values.get("google_204_ms"), ping_ms)) if values.get("google_204_ms") or ping_ms is not None else None,
        "isOnline": True,
    }

def same_endpoint(server, host, port, user):
    return (
        str(server.get("host", "")).strip().lower() == str(host or "").strip().lower()
        and int(server.get("sshPort") or 22) == int(port or 22)
        and str(server.get("sshUser", "")).strip() == str(user or "").strip()
    )

def is_monitor_server(config, server):
    return server.get("id") == config.get("monitorServerId") or same_endpoint(
        server,
        config.get("monitorHost"),
        config.get("monitorPort") or 22,
        config.get("monitorUser"),
    )

def bastion_is_monitor(config, server):
    host = (server.get("bastionHost") or "").strip()
    if not host:
        return False
    return (
        host.lower() == str(config.get("monitorHost") or "").strip().lower()
        and int(server.get("bastionPort") or 22) == int(config.get("monitorPort") or 22)
        and str(server.get("bastionUser") or server.get("sshUser") or "").strip() == str(config.get("monitorUser") or "").strip()
    )

def ssh_command(config, server):
    args = [
        "ssh",
        "-o", "BatchMode=yes",
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "ConnectTimeout=8",
        "-o", "ServerAliveInterval=5",
        "-o", "ServerAliveCountMax=1",
        "-p", str(server.get("sshPort") or 22),
    ]
    key_path = (server.get("sshKeyPath") or "").strip()
    if key_path:
        args += ["-i", key_path, "-o", "IdentitiesOnly=yes"]
    bastion_host = (server.get("bastionHost") or "").strip()
    if bastion_host and not bastion_is_monitor(config, server):
        bastion_user = (server.get("bastionUser") or server.get("sshUser") or "root").strip()
        bastion_port = int(server.get("bastionPort") or 22)
        bastion_key = (server.get("bastionSshKeyPath") or "").strip()
        if bastion_key:
            proxy = "ssh -W %h:%p -o BatchMode=yes -o StrictHostKeyChecking=accept-new -p {} -i {} -o IdentitiesOnly=yes {}".format(
                bastion_port,
                shlex.quote(bastion_key),
                shlex.quote(f"{bastion_user}@{bastion_host}"),
            )
            args += ["-o", f"ProxyCommand={proxy}"]
        else:
            args += ["-o", f"ProxyJump={bastion_user}@{bastion_host}:{bastion_port}"]
    args.append(f"{server.get('sshUser') or 'root'}@{server.get('host')}")
    return args

def run_metrics(config, server):
    started = time.monotonic()
    if is_monitor_server(config, server):
        result = subprocess.run(["/bin/sh", "-lc", METRICS_SCRIPT], capture_output=True, text=True, timeout=25)
    else:
        result = subprocess.run(ssh_command(config, server) + [METRICS_SCRIPT], capture_output=True, text=True, timeout=35)
    if result.returncode != 0:
        raise RuntimeError((result.stderr or result.stdout or "ssh failed").strip())
    return parse_metrics(server.get("id"), result.stdout, (time.monotonic() - started) * 1000.0)

def offline_point(server_id, previous=None):
    previous = previous or {}
    rx = int(previous.get("totalRxBytes") or previous.get("rxBytes") or 0)
    tx = int(previous.get("totalTxBytes") or previous.get("txBytes") or 0)
    return {
        "serverId": server_id,
        "timestamp": utc_now(),
        "cpuPercent": 0,
        "ramUsedMb": 0,
        "ramTotalMb": 0,
        "ramPercent": 0,
        "diskUsed": previous.get("diskUsed") or "--",
        "diskTotal": previous.get("diskTotal") or "--",
        "diskPercent": 0,
        "loadAverage": [0, 0, 0],
        "uptimeSec": 0,
        "uptime": "Offline",
        "rxBytes": rx,
        "txBytes": tx,
        "totalRxBytes": rx,
        "totalTxBytes": tx,
        "totalTrafficBytes": rx + tx,
        "pingMs": None,
        "isOnline": False,
    }

def push_event(events, level, kind, server, message):
    events.insert(0, {
        "id": f"{int(time.time() * 1000)}-{server.get('id')}-{kind}",
        "level": level,
        "kind": kind,
        "serverId": server.get("id"),
        "serverName": server.get("name"),
        "message": message,
        "timestamp": utc_now(),
    })
    del events[MAX_EVENTS:]

def main():
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    config = load_json(CONFIG_PATH, {})
    cache = load_json(METRICS_PATH, {})
    events = load_json(EVENTS_PATH, [])
    runtime = load_json(RUNTIME_PATH, {"downServers": [], "highCpuSince": {}, "highCpuAlerted": [], "failCounts": {}})
    down = set(runtime.get("downServers") or [])
    high_since = dict(runtime.get("highCpuSince") or {})
    high_alerted = set(runtime.get("highCpuAlerted") or [])
    fail_counts = dict(runtime.get("failCounts") or {})
    now = time.time()

    for server in config.get("servers") or []:
        server_id = server.get("id")
        if not server_id:
            continue
        history = cache.get(server_id) or []
        previous = history[-1] if history else None
        try:
            point = run_metrics(config, server)
            fail_counts.pop(server_id, None)
            if server_id in down:
                down.remove(server_id)
            if point.get("cpuPercent", 0) > 90:
                high_since.setdefault(server_id, now)
                if now - float(high_since.get(server_id) or now) >= 60 and server_id not in high_alerted:
                    high_alerted.add(server_id)
                    push_event(events, "warn", "cpu_high", server, f"{server.get('name')} CPU has been above 90% for more than 60 sec ({point.get('cpuPercent'):.1f}%)")
            else:
                high_since.pop(server_id, None)
                high_alerted.discard(server_id)
        except Exception as error:
            fail_count = int(fail_counts.get(server_id) or 0) + 1
            fail_counts[server_id] = fail_count
            if fail_count < 2:
                continue
            point = offline_point(server_id, previous)
            if server_id not in down:
                down.add(server_id)
                push_event(events, "error", "server_down", server, f"{server.get('name')} is unavailable from monitor: {error}")
            high_since.pop(server_id, None)
            high_alerted.discard(server_id)
        history.append(point)
        cache[server_id] = history[-MAX_POINTS_PER_SERVER:]

    runtime = {
        "downServers": sorted(down),
        "highCpuSince": high_since,
        "highCpuAlerted": sorted(high_alerted),
        "failCounts": fail_counts,
    }
    save_json(METRICS_PATH, cache)
    save_json(EVENTS_PATH, events[:MAX_EVENTS])
    save_json(RUNTIME_PATH, runtime)

if __name__ == "__main__":
    main()
"#
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[allow(dead_code)]
fn _temp_file_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metric(timestamp: DateTime<Utc>, is_online: bool, ping_ms: f64) -> Value {
        json!({
            "timestamp": timestamp.to_rfc3339(),
            "isOnline": is_online,
            "pingMs": ping_ms,
        })
    }

    #[test]
    fn stable_latest_metric_ignores_stale_history() {
        let history = vec![metric(Utc::now() - ChronoDuration::minutes(10), true, 12.0)];

        assert!(stable_latest_metric(&history, ChronoDuration::seconds(60)).is_none());
    }

    #[test]
    fn stable_latest_metric_softens_single_fresh_offline_sample() {
        let now = Utc::now();
        let history = vec![
            metric(now - ChronoDuration::seconds(20), true, 12.0),
            metric(now, false, 0.0),
        ];

        let selected = stable_latest_metric(&history, ChronoDuration::seconds(60))
            .expect("fresh history should select a metric");

        assert_eq!(selected.get("pingMs").and_then(Value::as_f64), Some(12.0));
    }

    #[test]
    fn stable_latest_metric_keeps_repeated_fresh_offline_sample() {
        let now = Utc::now();
        let history = vec![
            metric(now - ChronoDuration::seconds(30), true, 12.0),
            metric(now - ChronoDuration::seconds(10), false, 0.0),
            metric(now, false, 0.0),
        ];

        let selected = stable_latest_metric(&history, ChronoDuration::seconds(60))
            .expect("fresh history should select a metric");

        assert!(!metric_is_online(selected));
    }
}

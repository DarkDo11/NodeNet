use crate::{config, config::ServerConfig, keychain, ssh};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use directories::UserDirs;
use futures::future::join_all;
use reqwest::{header::HOST, Client, Method, RequestBuilder, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};
use tauri::{self, AppHandle};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.3x-ui";
const LEGACY_KEYCHAIN_SERVICE: &str = "vpnctrl.3x-ui";
const INBOUNDS_API: &str = "/panel/api/inbounds";
const XRAY_CONFIG_API: &str = "/panel/api/server/getConfigJson";
const XRAY_TEMPLATE_API: &str = "/panel/api/xray";
const XRAY_TEMPLATE_API_LEGACY: &str = "/panel/xray";
const XRAY_UPDATE_API: &str = "/panel/api/xray/update";
const XRAY_UPDATE_API_LEGACY: &str = "/panel/xray/update";
const SERVER_RESTART_XRAY_API: &str = "/panel/api/server/restartXrayService";
const DEFAULT_OUTBOUND_TEST_URL: &str = "https://www.google.com/generate_204";
const XUI_BIN_DIR: &str = "/usr/local/x-ui/bin";
const XUI_BIN_CONFIG_PATH: &str = "/usr/local/x-ui/bin/config.json";
const SESSION_TTL: Duration = Duration::from_secs(25 * 60);
const PANEL_TUNNEL_TTL: Duration = Duration::from_secs(30 * 60);

type SessionCache = Mutex<HashMap<String, (PanelSession, Instant)>>;
type PanelTunnelCache = Mutex<HashMap<String, CachedPanelTunnel>>;

static SESSION_CACHE: LazyLock<SessionCache> = LazyLock::new(|| Mutex::new(HashMap::new()));
static PANEL_TUNNELS: LazyLock<PanelTunnelCache> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PanelCredentials {
    username: String,
    password: String,
}

#[derive(Debug, Clone)]
pub struct PanelSession {
    request_base_url: String,
    client: Client,
    credentials: PanelCredentials,
    csrf_token: Option<String>,
    cache_key: Option<String>,
    tunnel_key: Option<String>,
    host_header: Option<String>,
    uses_bastion: bool,
}

#[derive(Debug)]
struct CachedPanelTunnel {
    tunnel: ssh::SshTunnel,
    last_used: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeXInbound {
    pub id: i64,
    pub remark: String,
    pub protocol: String,
    pub port: u16,
    pub enable: bool,
    pub up: u64,
    pub down: u64,
    pub total: u64,
    pub client_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeXClient {
    pub id: String,
    pub email: String,
    pub inbound_id: i64,
    pub inbound_remark: String,
    pub protocol: String,
    pub port: u16,
    pub enable: bool,
    pub status: String,
    pub up: u64,
    pub down: u64,
    pub total: u64,
    pub expiry_time: i64,
    pub expiry: String,
    pub used_percent: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiEnvelope<T> {
    success: Option<bool>,
    msg: Option<String>,
    obj: Option<T>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawInbound {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    remark: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    enable: bool,
    #[serde(default)]
    up: u64,
    #[serde(default)]
    down: u64,
    #[serde(default)]
    total: u64,
    #[serde(default)]
    settings: String,
    #[serde(default)]
    stream_settings: String,
    #[serde(default)]
    client_stats: Vec<RawClientTraffic>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawClientTraffic {
    #[serde(default)]
    email: String,
    #[serde(default)]
    up: u64,
    #[serde(default)]
    down: u64,
    #[serde(default)]
    total: u64,
    #[serde(default)]
    expiry_time: i64,
    #[serde(default)]
    enable: Option<bool>,
}

#[derive(Debug, Clone)]
struct XrayPanelSettings {
    config: Value,
    outbound_test_url: String,
}

pub async fn save_credentials(
    app: &AppHandle,
    server: &ServerConfig,
    username: &str,
    password: &str,
) -> Result<()> {
    let secret = serde_json::to_string(&PanelCredentials {
        username: username.trim().to_string(),
        password: password.to_string(),
    })
    .context("failed to serialize 3x-ui credentials")?;
    keychain::save_password(app, KEYCHAIN_SERVICE, &keychain_account(server), &secret).await
}

pub async fn delete_credentials(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    let account = keychain_account(server);
    // Delete from primary service first.  If that fails for a real reason
    // (not "not found"), propagate the error regardless of legacy outcome.
    let primary_result = keychain::delete_password(app, KEYCHAIN_SERVICE, &account).await;
    let legacy_result = keychain::delete_password(app, LEGACY_KEYCHAIN_SERVICE, &account).await;

    primary_result?;
    legacy_result
}

pub async fn read_saved_credentials(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<(String, String)>> {
    let Some(credentials) = read_credentials_optional(app, server).await? else {
        return Ok(None);
    };

    Ok(Some((credentials.username, credentials.password)))
}

pub async fn clear_server_cache(server: &ServerConfig) {
    let session_key = session_cache_key(server);
    SESSION_CACHE.lock().await.remove(&session_key);

    let tunnel_prefix = format!("{}|", server.id);
    PANEL_TUNNELS
        .lock()
        .await
        .retain(|key, _| !key.starts_with(&tunnel_prefix));
}

pub async fn get_inbounds(app: &AppHandle, server: &ServerConfig) -> Result<Vec<ThreeXInbound>> {
    let mut session = PanelSession::for_server(app, server).await?;
    let raw = get_raw_inbounds(&mut session).await?;
    Ok(raw.iter().map(map_inbound).collect())
}

/// Certificate files the panel's inbounds actually point Xray at (from each
/// inbound's `tlsSettings`/`xtlsSettings`), regardless of where they live on
/// disk. The panel's own "Apply" cert feature (acme.sh) commonly stores certs
/// outside `/etc/letsencrypt/live/`, so this is how the SSL tab finds those
/// too, not just plain Certbot installs.
pub async fn get_inbound_certificate_paths(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Vec<String>> {
    let mut session = PanelSession::for_server(app, server).await?;
    let raw = get_raw_inbounds(&mut session).await?;

    let mut paths = Vec::new();
    for inbound in &raw {
        let stream = parse_json_object(&inbound.stream_settings).unwrap_or_else(|| json!({}));
        for settings_key in ["tlsSettings", "xtlsSettings"] {
            let Some(certs) = stream
                .pointer(&format!("/{settings_key}/certificates"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            for cert in certs {
                if let Some(file) = cert.get("certificateFile").and_then(Value::as_str) {
                    let trimmed = file.trim();
                    if !trimmed.is_empty() {
                        paths.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub async fn get_xray_config(app: &AppHandle, server: &ServerConfig) -> Result<Value> {
    let mut session = PanelSession::for_server(app, server).await?;
    Ok(get_xray_panel_settings(&mut session).await?.config)
}

pub async fn save_xray_config(
    app: &AppHandle,
    server: &ServerConfig,
    mut config: Value,
) -> Result<()> {
    ensure_api_routing_rule(&mut config);
    let mut session = PanelSession::for_server(app, server).await?;
    if let Err(panel_error) = save_xray_config_with_session(&mut session, &config).await {
        save_xray_config_via_ssh(app, server, &config)
            .await
            .with_context(|| {
                format!("3x-ui Xray settings API failed ({panel_error}); SSH fallback also failed")
            })?;
    }
    restart_xray_with_session(app, server, &mut session).await
}

async fn get_xray_panel_settings(session: &mut PanelSession) -> Result<XrayPanelSettings> {
    let template_error = match session.post_value(XRAY_TEMPLATE_API).await {
        Ok(response) => return parse_xray_panel_settings(response),
        Err(error) => error,
    };
    // 3x-ui < v3 served the Xray template at /panel/xray instead of /panel/api/xray.
    let legacy_error = match session.post_value(XRAY_TEMPLATE_API_LEGACY).await {
        Ok(response) => return parse_xray_panel_settings(response),
        Err(error) => error,
    };
    let response = session.get_value(XRAY_CONFIG_API).await.with_context(|| {
        format!(
            "3x-ui Xray template API failed ({template_error}); legacy API also failed ({legacy_error})"
        )
    })?;
    parse_xray_panel_settings(response)
}

async fn save_xray_config_with_session(session: &mut PanelSession, config: &Value) -> Result<()> {
    let outbound_test_url = get_xray_panel_settings(session)
        .await
        .context("failed to read current Xray panel settings before save")?
        .outbound_test_url;
    let xray_setting = serde_json::to_string(config).context("failed to serialize Xray config")?;
    let form = vec![
        ("xraySetting", xray_setting),
        ("outboundTestUrl", outbound_test_url),
    ];
    match session.post_form_action(XRAY_UPDATE_API, &form).await {
        Ok(()) => Ok(()),
        // 3x-ui < v3 served the Xray update API at /panel/xray/update instead of /panel/api/xray/update.
        Err(_) => session.post_form_action(XRAY_UPDATE_API_LEGACY, &form).await,
    }
}

async fn save_xray_config_via_ssh(
    app: &AppHandle,
    server: &ServerConfig,
    config: &Value,
) -> Result<()> {
    let raw = serde_json::to_string(config).context("failed to serialize Xray config")?;
    let local_path =
        std::env::temp_dir().join(format!("nodenet-xray-template-{}.json", Uuid::new_v4()));
    fs::write(&local_path, raw)
        .with_context(|| format!("failed to write {}", local_path.display()))?;

    let remote_path = format!("/tmp/nodenet-xray-template-{}.json", Uuid::new_v4());
    let upload_result = ssh::upload_file(app, server, &local_path, &remote_path).await;
    let _ = fs::remove_file(&local_path);
    upload_result?;

    let command = format!(
        "ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/;s/armv7l/arm32/') && cd /usr/local/x-ui && ./bin/xray-linux-$ARCH -test -config {} >/dev/null && python3 -c {} {} && rm -f {} && (systemctl restart x-ui || systemctl restart 3x-ui || systemctl restart xray)",
        shell_single_quote(&remote_path),
        shell_single_quote(
            "import sqlite3,sys; value=open(sys.argv[1],encoding='utf-8').read(); db=sqlite3.connect('/etc/x-ui/x-ui.db'); db.execute('delete from settings where key=?', ('xrayTemplateConfig',)); db.execute('insert into settings(key,value) values(?,?)', ('xrayTemplateConfig', value)); db.commit(); db.close()"
        ),
        shell_single_quote(&remote_path),
        shell_single_quote(&remote_path),
    );
    ssh::execute_combined(app, server, &command, 60).await?;
    Ok(())
}

fn parse_xray_panel_settings(response: Value) -> Result<XrayPanelSettings> {
    let value = decode_json_value(response).context("failed to parse Xray settings response")?;
    let outbound_test_url = value
        .get("outboundTestUrl")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_OUTBOUND_TEST_URL)
        .to_string();

    let config = if is_xray_config(&value) {
        value
    } else if let Some(setting) = value.get("xraySetting") {
        decode_json_value(setting.clone()).context("failed to parse xraySetting")?
    } else {
        bail!("3x-ui Xray settings response did not include xraySetting");
    };

    if !is_xray_config(&config) {
        bail!("3x-ui xraySetting does not look like an Xray config");
    }

    Ok(XrayPanelSettings {
        config,
        outbound_test_url,
    })
}

fn decode_json_value(value: Value) -> Result<Value> {
    match value {
        Value::String(raw) => serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("failed to parse JSON string: {}", raw.trim())),
        other => Ok(other),
    }
}

fn is_xray_config(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    [
        "inbounds",
        "outbounds",
        "routing",
        "api",
        "dns",
        "log",
        "policy",
        "stats",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}

fn ensure_api_routing_rule(config: &mut Value) {
    let Some(config_object) = config.as_object_mut() else {
        return;
    };

    let routing = config_object
        .entry("routing")
        .or_insert_with(|| json!({ "domainStrategy": "AsIs", "rules": [] }));
    if !routing.is_object() {
        *routing = json!({ "domainStrategy": "AsIs", "rules": [] });
    }

    let Some(routing_object) = routing.as_object_mut() else {
        return;
    };
    let rules = routing_object
        .entry("rules")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !rules.is_array() {
        *rules = Value::Array(Vec::new());
    }

    let Some(rule_values) = rules.as_array_mut() else {
        return;
    };
    let api_rule_index = rule_values.iter().position(is_api_routing_rule);
    let api_rule = api_rule_index
        .map(|index| rule_values.remove(index))
        .unwrap_or_else(|| {
            json!({
                "type": "field",
                "inboundTag": ["api"],
                "outboundTag": "api",
            })
        });
    rule_values.insert(0, api_rule);
}

fn is_api_routing_rule(rule: &Value) -> bool {
    let Some(object) = rule.as_object() else {
        return false;
    };
    if object.get("outboundTag").and_then(Value::as_str) != Some("api") {
        return false;
    }
    match object.get("inboundTag") {
        Some(Value::String(tag)) => tag == "api",
        Some(Value::Array(tags)) => tags.iter().any(|tag| tag.as_str() == Some("api")),
        _ => false,
    }
}

pub async fn get_clients(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
) -> Result<Vec<ThreeXClient>> {
    let mut session = PanelSession::for_server(app, server).await?;
    let inbound = get_raw_inbound(&mut session, inbound_id).await?;
    Ok(map_clients(&inbound))
}

pub async fn add_client(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
    name: String,
    limit_gb: f64,
    expire_days: i64,
) -> Result<ThreeXClient> {
    let mut session = PanelSession::for_server(app, server).await?;
    let inbound = get_raw_inbound(&mut session, inbound_id).await?;
    let client_id = Uuid::new_v4().to_string();
    let sub_id = Uuid::new_v4().simple().to_string();
    let total_gb = if limit_gb <= 0.0 || !limit_gb.is_finite() {
        0
    } else {
        (limit_gb * 1024.0 * 1024.0 * 1024.0).round() as u64
    };
    let expiry_time = if expire_days <= 0 {
        0
    } else {
        (Utc::now() + ChronoDuration::days(expire_days)).timestamp_millis()
    };
    let email = name.trim().to_string();

    if email.is_empty() {
        bail!("client name cannot be empty");
    }

    let mut client = json!({
        "email": email,
        "limitIp": 0,
        "totalGB": total_gb,
        "expiryTime": expiry_time,
        "enable": true,
        "tgId": "",
        "subId": sub_id,
        "comment": "",
        "reset": 0
    });

    match inbound.protocol.to_ascii_lowercase().as_str() {
        "trojan" => {
            client["password"] = json!(client_id);
            client["flow"] = json!("");
        }
        "shadowsocks" => {
            // Shadowsocks clients use email as identifier and inherit the
            // inbound cipher+password — they have no per-client credentials.
            bail!(
                "Adding clients to a Shadowsocks inbound is not supported: \
                 Shadowsocks inbounds share a single password defined on the inbound itself."
            );
        }
        _ => {
            client["id"] = json!(client_id);
            client["alterId"] = json!(0);
            client["flow"] = json!(default_flow(&inbound));
        }
    }

    let settings = json!({ "clients": [client.clone()] });
    let payload = json!({
        "id": inbound_id,
        "settings": serde_json::to_string(&settings)?
    });

    session
        .post_action(&format!("{INBOUNDS_API}/addClient"), Some(payload))
        .await?;

    let refreshed = get_raw_inbound(&mut session, inbound_id)
        .await
        .unwrap_or(inbound);
    map_clients(&refreshed)
        .into_iter()
        .find(|item| item.id == client_identifier(&refreshed.protocol, &client))
        .ok_or_else(|| anyhow!("client was added but was not returned by 3x-ui"))
}

pub async fn delete_client(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
    client_id: String,
) -> Result<()> {
    let mut session = PanelSession::for_server(app, server).await?;
    session
        .post_action(
            &format!(
                "{INBOUNDS_API}/{}/delClient/{}",
                inbound_id,
                encode_component(&client_id)
            ),
            None,
        )
        .await
}

pub async fn reset_all_expired_clients(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
) -> Result<usize> {
    let clients = get_clients(app, server, inbound_id).await?;
    let expired = clients
        .into_iter()
        .filter(|client| client.status == "expired")
        .collect::<Vec<_>>();
    let total = expired.len();

    let futures = expired
        .iter()
        .map(|client| reset_client_traffic(app, server, inbound_id, client.id.clone()));
    for result in join_all(futures).await {
        result?;
    }

    Ok(total)
}

pub async fn delete_all_disabled_clients(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
) -> Result<usize> {
    let clients = get_clients(app, server, inbound_id).await?;
    let disabled = clients
        .into_iter()
        .filter(|client| client.status == "disabled")
        .collect::<Vec<_>>();
    let total = disabled.len();

    let futures = disabled
        .iter()
        .map(|client| delete_client(app, server, inbound_id, client.id.clone()));
    for result in join_all(futures).await {
        result?;
    }

    Ok(total)
}

pub async fn reset_client_traffic(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
    client_id: String,
) -> Result<()> {
    let mut session = PanelSession::for_server(app, server).await?;
    let inbound = get_raw_inbound(&mut session, inbound_id).await?;
    let client = find_client_value(&inbound, &client_id)?;
    let email = string_field(&client, "email").context("client email is missing")?;

    session
        .post_action(
            &format!(
                "{INBOUNDS_API}/{}/resetClientTraffic/{}",
                inbound_id,
                encode_component(&email)
            ),
            None,
        )
        .await
}

pub async fn extend_client(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
    client_id: String,
    days: i64,
) -> Result<ThreeXClient> {
    if days <= 0 {
        bail!("extend days must be positive");
    }

    let mut session = PanelSession::for_server(app, server).await?;
    let inbound = get_raw_inbound(&mut session, inbound_id).await?;
    let mut client = find_client_value(&inbound, &client_id)?;
    let current_expiry = number_field(&client, "expiryTime").unwrap_or(0);
    let base_expiry = current_expiry.max(Utc::now().timestamp_millis());
    let delta_ms = days
        .checked_mul(86_400_000)
        .and_then(|d| base_expiry.checked_add(d))
        .ok_or_else(|| anyhow::anyhow!("days value too large, would overflow expiry timestamp"))?;
    client["expiryTime"] = json!(delta_ms);

    let settings = json!({ "clients": [client.clone()] });
    let payload = json!({
        "id": inbound_id,
        "settings": serde_json::to_string(&settings)?
    });

    session
        .post_action(
            &format!(
                "{INBOUNDS_API}/updateClient/{}",
                encode_component(&client_id)
            ),
            Some(payload),
        )
        .await?;

    let refreshed = get_raw_inbound(&mut session, inbound_id)
        .await
        .unwrap_or(inbound);
    map_clients(&refreshed)
        .into_iter()
        .find(|item| item.id == client_identifier(&refreshed.protocol, &client))
        .ok_or_else(|| anyhow!("client was extended but was not returned by 3x-ui"))
}

pub async fn generate_link(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
    client_id: String,
) -> Result<String> {
    let mut session = PanelSession::for_server(app, server).await?;
    let inbound = get_raw_inbound(&mut session, inbound_id).await?;
    let client = find_client_value(&inbound, &client_id)?;
    generate_link_for_client(server, &inbound, &client)
}

pub async fn restart_xray(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    let mut session = PanelSession::for_server(app, server).await?;
    restart_xray_with_session(app, server, &mut session).await
}

async fn restart_xray_with_session(
    app: &AppHandle,
    server: &ServerConfig,
    session: &mut PanelSession,
) -> Result<()> {
    match session.post_action(SERVER_RESTART_XRAY_API, None).await {
        Ok(()) => Ok(()),
        Err(panel_error) => {
            ssh::execute(
                app,
                server,
                "systemctl restart x-ui || systemctl restart 3x-ui || systemctl restart xray",
            )
            .await
            .with_context(|| {
                format!("3x-ui restart API failed ({panel_error}); SSH fallback also failed")
            })?;
            Ok(())
        }
    }
}

pub async fn reboot_server(app: &AppHandle, server: &ServerConfig) -> Result<()> {
    ssh::execute(
        app,
        server,
        "nohup sh -c 'sleep 1; reboot' >/dev/null 2>&1 &",
    )
    .await?;
    Ok(())
}

pub async fn download_config(app: &AppHandle, server: &ServerConfig) -> Result<String> {
    let directory = config::config_dir()?.join("backups");
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create backup directory {}", directory.display()))?;
    let filename = format!(
        "{}-3x-ui-config-{}.json",
        server.id,
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = directory.join(filename);
    ssh::download_file(app, server, XUI_BIN_CONFIG_PATH, &path).await?;
    app.opener()
        .reveal_item_in_dir(&path)
        .context("failed to reveal backup in Finder")?;
    Ok(path.display().to_string())
}

pub async fn upload_routing_file(
    app: &AppHandle,
    server: &ServerConfig,
    local_path: &str,
    remote_filename: Option<String>,
) -> Result<String> {
    let local_path = expand_local_path(local_path);
    let filename = validate_routing_filename(remote_filename.as_deref(), &local_path)?;
    let remote_path = format!("{XUI_BIN_DIR}/{filename}");
    let tmp_path = format!("{XUI_BIN_DIR}/.nodenet-upload-{filename}.tmp");

    ssh::upload_file(app, server, &local_path, &tmp_path).await?;
    let command = format!(
        "chmod 644 {} && mv {} {}",
        shell_single_quote(&tmp_path),
        shell_single_quote(&tmp_path),
        shell_single_quote(&remote_path)
    );
    ssh::execute_combined(app, server, &command, 60).await?;
    ssh::execute_combined(
        app,
        server,
        "systemctl restart x-ui || systemctl restart 3x-ui || systemctl restart xray",
        60,
    )
    .await?;

    Ok(remote_path)
}

pub async fn export_clients_csv(
    app: &AppHandle,
    server: &ServerConfig,
    inbound_id: i64,
) -> Result<String> {
    let clients = get_clients(app, server, inbound_id).await?;
    let downloads = UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|path| path.to_path_buf()))
        .context("unable to resolve Downloads directory")?;
    let path = downloads.join(format!(
        "nodenet-clients-{}.csv",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    let mut csv = String::from("email,up,down,total,expiry,status\n");

    for client in clients {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_cell(&client.email),
            client.up,
            client.down,
            client.total,
            csv_cell(&client.expiry),
            csv_cell(&client.status)
        ));
    }

    fs::write(&path, csv).with_context(|| format!("failed to write {}", path.display()))?;
    app.opener()
        .open_path(path.display().to_string(), None::<String>)
        .context("failed to open exported CSV")?;
    Ok(path.display().to_string())
}

struct PanelTransport {
    request_base_url: String,
    client: Client,
    tunnel_key: Option<String>,
    host_header: Option<String>,
    uses_bastion: bool,
}

async fn login_for_server(
    app: &AppHandle,
    server: &ServerConfig,
    credentials: PanelCredentials,
    cache_key: String,
) -> Result<PanelSession> {
    let panel_url = server
        .panel_url
        .as_deref()
        .context("panelUrl is not configured for this server")?;
    let transport = build_panel_transport(app, server, panel_url).await?;

    let csrf_token = match login_with_client(
        &transport.client,
        &transport.request_base_url,
        &credentials,
        transport.host_header.as_deref(),
        transport.uses_bastion,
    )
    .await
    {
        Ok(csrf_token) => csrf_token,
        Err(error) => {
            invalidate_panel_tunnel(transport.tunnel_key.as_deref()).await;
            return Err(error);
        }
    };

    Ok(PanelSession {
        request_base_url: transport.request_base_url,
        client: transport.client,
        credentials,
        csrf_token,
        cache_key: Some(cache_key),
        tunnel_key: transport.tunnel_key,
        host_header: transport.host_header,
        uses_bastion: transport.uses_bastion,
    })
}

async fn build_panel_transport(
    app: &AppHandle,
    server: &ServerConfig,
    panel_url: &str,
) -> Result<PanelTransport> {
    let base_url = normalize_panel_url(panel_url)?;
    let parsed = Url::parse(&base_url).context("panelUrl is invalid")?;
    let mut builder = Client::builder()
        .cookie_store(true)
        .danger_accept_invalid_certs(!server.ssl_verify)
        .timeout(Duration::from_secs(16));

    let mut request_base_url = base_url.clone();
    let mut tunnel_key = None;
    let mut host_header = None;
    let uses_bastion = ssh::has_bastion(server);
    if uses_bastion {
        let host = parsed
            .host_str()
            .context("panelUrl must include a host for bastion tunnel")?
            .to_string();
        let port = parsed
            .port_or_known_default()
            .context("panelUrl must include a port or http/https scheme")?;
        let key = panel_tunnel_key(server, &base_url, &host, port);
        let local_port = get_or_open_panel_tunnel(app, server, &key, &host, port)
            .await
            .with_context(|| format!("Cannot reach 3x-ui panel through bastion ({host}:{port})"))?;

        if parsed.scheme() == "http" || !server.ssl_verify {
            request_base_url = local_tunnel_url(&parsed, local_port)?;
            host_header = Some(host_header_value(&parsed, &host, port));
        } else {
            if host.parse::<std::net::IpAddr>().is_ok() {
                bail!(
                    "3x-ui over bastion with strict TLS requires a panel hostname, or disabled SSL verification"
                );
            }
            // Keep strict HTTPS requests addressed to the original panel host,
            // preserving Host, TLS SNI, and certificate validation, while TCP
            // connects to the local bastion tunnel endpoint.
            builder = builder.resolve(&host, SocketAddr::from(([127, 0, 0, 1], local_port)));
        }
        tunnel_key = Some(key);
    }

    let client = builder.build().with_context(|| {
        if uses_bastion {
            "failed to create 3x-ui http client through bastion"
        } else {
            "failed to create 3x-ui http client"
        }
    })?;

    Ok(PanelTransport {
        request_base_url,
        client,
        tunnel_key,
        host_header,
        uses_bastion,
    })
}

async fn get_or_open_panel_tunnel(
    app: &AppHandle,
    server: &ServerConfig,
    key: &str,
    target_host: &str,
    target_port: u16,
) -> Result<u16> {
    let mut tunnels = PANEL_TUNNELS.lock().await;
    cleanup_stale_panel_tunnels(&mut tunnels);

    if let Some(cached) = tunnels.get_mut(key) {
        cached.last_used = Instant::now();
        return Ok(cached.tunnel.local_port());
    }

    let tunnel = ssh::open_bastion_tunnel(app, server, target_host, target_port)
        .await
        .context("Bastion tunnel failed")?;
    let local_port = tunnel.local_port();
    tunnels.insert(
        key.to_string(),
        CachedPanelTunnel {
            tunnel,
            last_used: Instant::now(),
        },
    );
    Ok(local_port)
}

fn cleanup_stale_panel_tunnels(tunnels: &mut HashMap<String, CachedPanelTunnel>) {
    let now = Instant::now();
    tunnels.retain(|_, tunnel| now.duration_since(tunnel.last_used) < PANEL_TUNNEL_TTL);
}

pub fn start_panel_tunnel_reaper() {
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(PANEL_TUNNEL_TTL / 2).await;
            let mut tunnels = PANEL_TUNNELS.lock().await;
            cleanup_stale_panel_tunnels(&mut tunnels);
        }
    });
}

async fn touch_panel_tunnel(key: Option<&str>) -> bool {
    let Some(key) = key else {
        return true;
    };
    let mut tunnels = PANEL_TUNNELS.lock().await;
    cleanup_stale_panel_tunnels(&mut tunnels);
    if let Some(tunnel) = tunnels.get_mut(key) {
        tunnel.last_used = Instant::now();
        true
    } else {
        false
    }
}

async fn invalidate_panel_tunnel(key: Option<&str>) {
    if let Some(key) = key {
        PANEL_TUNNELS.lock().await.remove(key);
    }
}

impl PanelSession {
    async fn for_server(app: &AppHandle, server: &ServerConfig) -> Result<Self> {
        let cache_key = session_cache_key(server);
        let cached = { SESSION_CACHE.lock().await.get(&cache_key).cloned() };
        if let Some((session, cached_at)) = cached {
            if cached_at.elapsed() < SESSION_TTL
                && touch_panel_tunnel(session.tunnel_key.as_deref()).await
            {
                return Ok(session);
            }
            SESSION_CACHE.lock().await.remove(&cache_key);
        }

        let credentials = read_credentials(app, server).await?;
        let session = login_for_server(app, server, credentials, cache_key.clone()).await?;
        SESSION_CACHE
            .lock()
            .await
            .insert(cache_key, (session.clone(), Instant::now()));
        Ok(session)
    }

    async fn relogin(&mut self) -> Result<()> {
        self.csrf_token = login_with_client(
            &self.client,
            &self.request_base_url,
            &self.credentials,
            self.host_header.as_deref(),
            self.uses_bastion,
        )
        .await?;
        Ok(())
    }

    async fn refresh_cache(&self) {
        if let Some(cache_key) = &self.cache_key {
            SESSION_CACHE
                .lock()
                .await
                .insert(cache_key.clone(), (self.clone(), Instant::now()));
        }
    }

    async fn relogin_and_refresh(&mut self) -> Result<()> {
        self.invalidate_cache().await;
        self.relogin().await?;
        self.refresh_cache().await;
        Ok(())
    }

    async fn invalidate_cache(&self) {
        if let Some(cache_key) = &self.cache_key {
            SESSION_CACHE.lock().await.remove(cache_key);
        }
    }

    async fn invalidate_transport(&self) {
        self.invalidate_cache().await;
        invalidate_panel_tunnel(self.tunnel_key.as_deref()).await;
    }

    async fn get_obj<T>(&mut self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_obj(Method::GET, path, None).await
    }

    async fn post_value(&mut self, path: &str) -> Result<Value> {
        self.request_value(Method::POST, path, None).await
    }

    async fn get_value(&mut self, path: &str) -> Result<Value> {
        self.request_value(Method::GET, path, None).await
    }

    async fn post_action(&mut self, path: &str, body: Option<Value>) -> Result<()> {
        for attempt in 0..=1 {
            let mut request = self.request(Method::POST, path);
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin_and_refresh().await?;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui action failed ({status}): {}", text.trim());
            }

            if text.trim().is_empty() {
                return Ok(());
            }

            let envelope = parse_envelope::<Value>(&text)?;
            if envelope.success == Some(false) {
                if should_retry_login(envelope.msg.as_deref()) && attempt == 0 {
                    self.relogin_and_refresh().await?;
                    continue;
                }
                bail!(
                    "3x-ui action failed: {}",
                    envelope.msg.unwrap_or_else(|| "unknown error".to_string())
                );
            }

            return Ok(());
        }

        bail!("3x-ui action failed after relogin")
    }

    async fn post_form_action(&mut self, path: &str, form: &[(&str, String)]) -> Result<()> {
        for attempt in 0..=1 {
            let request = self.request(Method::POST, path).form(form);

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin_and_refresh().await?;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui action failed ({status}): {}", text.trim());
            }

            if text.trim().is_empty() {
                return Ok(());
            }

            let envelope = parse_envelope::<Value>(&text)?;
            if envelope.success == Some(false) {
                if should_retry_login(envelope.msg.as_deref()) && attempt == 0 {
                    self.relogin_and_refresh().await?;
                    continue;
                }
                bail!(
                    "3x-ui action failed: {}",
                    envelope.msg.unwrap_or_else(|| "unknown error".to_string())
                );
            }

            return Ok(());
        }

        bail!("3x-ui action failed after relogin")
    }

    async fn request_obj<T>(&mut self, method: Method, path: &str, body: Option<Value>) -> Result<T>
    where
        T: DeserializeOwned,
    {
        for attempt in 0..=1 {
            let mut request = self.request(method.clone(), path);
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin_and_refresh().await?;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui request failed ({status}): {}", text.trim());
            }

            let envelope = parse_envelope::<T>(&text)?;
            if envelope.success == Some(false) {
                if should_retry_login(envelope.msg.as_deref()) && attempt == 0 {
                    self.relogin_and_refresh().await?;
                    continue;
                }
                bail!(
                    "3x-ui request failed: {}",
                    envelope.msg.unwrap_or_else(|| "unknown error".to_string())
                );
            }

            return envelope.obj.context("3x-ui response did not include obj");
        }

        bail!("3x-ui request failed after relogin")
    }

    async fn request_value(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        for attempt in 0..=1 {
            let mut request = self.request(method.clone(), path);
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin_and_refresh().await?;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui request failed ({status}): {}", text.trim());
            }

            let value = serde_json::from_str::<Value>(&text)
                .with_context(|| format!("failed to parse 3x-ui response: {}", text.trim()))?;
            if let Some(success) = value.get("success").and_then(Value::as_bool) {
                if !success {
                    let message = value
                        .get("msg")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    if should_retry_login(Some(message)) && attempt == 0 {
                        self.relogin_and_refresh().await?;
                        continue;
                    }
                    bail!("3x-ui request failed: {message}");
                }
                return value
                    .get("obj")
                    .cloned()
                    .context("3x-ui response did not include obj");
            }

            return Ok(value);
        }

        bail!("3x-ui request failed after relogin")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.request_base_url, path)
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let include_csrf =
            method != Method::GET && method != Method::HEAD && method != Method::OPTIONS;
        let mut request = apply_host_header(
            self.client.request(method, self.url(path)),
            self.host_header.as_deref(),
        )
        .header("X-Requested-With", "XMLHttpRequest");
        if include_csrf {
            if let Some(csrf_token) = &self.csrf_token {
                request = request.header("X-CSRF-Token", csrf_token);
            }
        }
        request
    }

    async fn send_request(&self, request: RequestBuilder, path: &str) -> Result<reqwest::Response> {
        match request.send().await {
            Ok(response) => Ok(response),
            Err(error) => {
                self.invalidate_transport().await;
                Err(panel_send_error(
                    error,
                    self.uses_bastion,
                    &format!("3x-ui request failed: {path}"),
                ))
            }
        }
    }
}

async fn login_with_client(
    client: &Client,
    request_base_url: &str,
    credentials: &PanelCredentials,
    host_header: Option<&str>,
    uses_bastion: bool,
) -> Result<Option<String>> {
    let csrf_token = fetch_csrf_token(client, request_base_url, host_header).await?;
    let form = [
        ("username", credentials.username.clone()),
        ("password", credentials.password.clone()),
    ];
    let mut request = apply_host_header(
        client.post(format!("{request_base_url}/login")),
        host_header,
    )
    .header("X-Requested-With", "XMLHttpRequest")
    .form(&form);
    if let Some(csrf_token) = &csrf_token {
        request = request.header("X-CSRF-Token", csrf_token);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return Err(panel_send_error(
                error,
                uses_bastion,
                "3x-ui login request failed",
            ));
        }
    };
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        if uses_bastion {
            bail!(
                "3x-ui login failed through bastion ({status}): {}",
                text.trim()
            );
        }
        bail!("3x-ui login failed ({status}): {}", text.trim());
    }

    if text.trim().is_empty() {
        return Ok(csrf_token);
    }

    let envelope = parse_envelope::<Value>(&text)?;
    if envelope.success == Some(false) {
        if uses_bastion {
            bail!(
                "3x-ui login failed through bastion: {}",
                envelope
                    .msg
                    .unwrap_or_else(|| "invalid credentials".to_string())
            );
        }
        bail!(
            "3x-ui login failed: {}",
            envelope
                .msg
                .unwrap_or_else(|| "invalid credentials".to_string())
        );
    }

    Ok(csrf_token)
}

async fn fetch_csrf_token(
    client: &Client,
    request_base_url: &str,
    host_header: Option<&str>,
) -> Result<Option<String>> {
    let response = apply_host_header(client.get(format!("{request_base_url}/")), host_header)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .context("failed to fetch 3x-ui login page")?;
    if response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        if let Some(token) = csrf_token_from_html(&text) {
            return Ok(Some(token));
        }
    }

    let response = apply_host_header(
        client.get(format!("{request_base_url}/csrf-token")),
        host_header,
    )
    .header("X-Requested-With", "XMLHttpRequest")
    .send()
    .await
    .context("failed to fetch 3x-ui CSRF token")?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let text = response.text().await.unwrap_or_default();
    let envelope = parse_envelope::<String>(&text)?;
    Ok(envelope.obj.filter(|token| !token.trim().is_empty()))
}

fn csrf_token_from_html(html: &str) -> Option<String> {
    // Try double-quoted form first, then single-quoted.
    let (start, closing_quote) = if let Some(pos) = html.find(r#"name="csrf-token" content=""#) {
        let marker = r#"name="csrf-token" content=""#;
        (pos + marker.len(), '"')
    } else if let Some(pos) = html.find("name=\"csrf-token\" content='") {
        let marker = "name=\"csrf-token\" content='";
        (pos + marker.len(), '\'')
    } else if let Some(pos) = html.find("name='csrf-token' content=\"") {
        let marker = "name='csrf-token' content=\"";
        (pos + marker.len(), '"')
    } else if let Some(pos) = html.find("name='csrf-token' content='") {
        let marker = "name='csrf-token' content='";
        (pos + marker.len(), '\'')
    } else {
        return None;
    };
    let end = html[start..].find(closing_quote)?;
    let token = html[start..start + end].trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

async fn read_credentials(app: &AppHandle, server: &ServerConfig) -> Result<PanelCredentials> {
    read_credentials_optional(app, server)
        .await?
        .context("3x-ui password is not saved in Keychain")
}

async fn read_credentials_optional(
    app: &AppHandle,
    server: &ServerConfig,
) -> Result<Option<PanelCredentials>> {
    let account = keychain_account(server);
    let raw = match keychain::read_password(app, KEYCHAIN_SERVICE, &account).await? {
        Some(raw) => raw,
        None => match keychain::read_password(app, LEGACY_KEYCHAIN_SERVICE, &account).await? {
            Some(raw) => raw,
            None => return Ok(None),
        },
    };

    if raw.starts_with('{') {
        return serde_json::from_str::<PanelCredentials>(&raw)
            .context("failed to parse 3x-ui credentials from Keychain")
            .map(Some);
    }

    Ok(Some(PanelCredentials {
        username: server
            .panel_user
            .clone()
            .unwrap_or_else(|| "admin".to_string()),
        password: raw,
    }))
}

async fn get_raw_inbounds(session: &mut PanelSession) -> Result<Vec<RawInbound>> {
    session.get_obj(&format!("{INBOUNDS_API}/list")).await
}

async fn get_raw_inbound(session: &mut PanelSession, inbound_id: i64) -> Result<RawInbound> {
    match session
        .get_obj::<RawInbound>(&format!("{INBOUNDS_API}/get/{inbound_id}"))
        .await
    {
        Ok(inbound) => Ok(inbound),
        Err(_) => get_raw_inbounds(session)
            .await?
            .into_iter()
            .find(|item| item.id == inbound_id)
            .with_context(|| format!("inbound {inbound_id} was not found")),
    }
}

fn map_inbound(raw: &RawInbound) -> ThreeXInbound {
    ThreeXInbound {
        id: raw.id,
        remark: raw.remark.clone(),
        protocol: raw.protocol.clone(),
        port: raw.port,
        enable: raw.enable,
        up: raw.up,
        down: raw.down,
        total: raw.total,
        client_count: settings_clients(&raw.settings)
            .map(|clients| clients.len())
            .unwrap_or(raw.client_stats.len()),
    }
}

fn map_clients(raw: &RawInbound) -> Vec<ThreeXClient> {
    let stats_by_email = raw
        .client_stats
        .iter()
        .map(|traffic| (traffic.email.clone(), traffic))
        .collect::<HashMap<_, _>>();

    settings_clients(&raw.settings)
        .unwrap_or_default()
        .into_iter()
        .map(|client| {
            let email = string_field(&client, "email").unwrap_or_else(|| "unnamed".to_string());
            let traffic = stats_by_email.get(&email);
            let up = traffic.map(|item| item.up).unwrap_or(0);
            let down = traffic.map(|item| item.down).unwrap_or(0);
            let total = number_field(&client, "totalGB")
                .map(|value| {
                    let raw = value.max(0) as u64;
                    // Older panel versions store limit in actual GB; newer ones
                    // (and our own add_client) store bytes.  Values under 1 000 are
                    // almost certainly in GB — the smallest sensible byte limit
                    // would be a few MB at minimum.
                    if raw > 0 && raw < 1_000 {
                        raw * 1_073_741_824
                    } else {
                        raw
                    }
                })
                .or_else(|| traffic.map(|item| item.total))
                .unwrap_or(0);
            let expiry_time = number_field(&client, "expiryTime")
                .or_else(|| traffic.map(|item| item.expiry_time))
                .unwrap_or(0);
            let enable = bool_field(&client, "enable")
                .unwrap_or_else(|| traffic.and_then(|item| item.enable).unwrap_or(true));
            let used = up.saturating_add(down);
            let used_percent = if total == 0 {
                0.0
            } else {
                ((used as f64 / total as f64) * 100.0).min(100.0)
            };

            ThreeXClient {
                id: client_identifier(&raw.protocol, &client),
                email,
                inbound_id: raw.id,
                inbound_remark: raw.remark.clone(),
                protocol: raw.protocol.clone(),
                port: raw.port,
                enable,
                status: client_status(enable, total, used, expiry_time),
                up,
                down,
                total,
                expiry_time,
                expiry: format_expiry(expiry_time),
                used_percent,
            }
        })
        .collect()
}

fn generate_link_for_client(
    server: &ServerConfig,
    inbound: &RawInbound,
    client: &Value,
) -> Result<String> {
    let protocol = inbound.protocol.to_ascii_lowercase();
    let stream = parse_json_object(&inbound.stream_settings).unwrap_or_else(|| json!({}));
    let network = string_path(&stream, &["network"]).unwrap_or_else(|| "tcp".to_string());
    let security = string_path(&stream, &["security"]).unwrap_or_else(|| "none".to_string());
    let email = string_field(client, "email").unwrap_or_else(|| inbound.remark.clone());
    let address = server.host.clone();

    match protocol.as_str() {
        "vless" => {
            let id = string_field(client, "id").context("vless client id is missing")?;
            let mut params = common_link_params(client, &stream, &network, &security);
            let query = build_query(&mut params);
            Ok(format!(
                "vless://{}@{}:{}{}#{}",
                id,
                address,
                inbound.port,
                query,
                encode_component(&email)
            ))
        }
        "trojan" => {
            let password =
                string_field(client, "password").context("trojan client password is missing")?;
            let mut params = common_link_params(client, &stream, &network, &security);
            let query = build_query(&mut params);
            Ok(format!(
                "trojan://{}@{}:{}{}#{}",
                encode_component(&password),
                address,
                inbound.port,
                query,
                encode_component(&email)
            ))
        }
        "vmess" => {
            let id = string_field(client, "id").context("vmess client id is missing")?;
            let host = ws_host(&stream).unwrap_or_default();
            let path = ws_path(&stream).unwrap_or_default();
            let sni = tls_sni(&stream).unwrap_or_default();
            let vmess = json!({
                "v": "2",
                "ps": email,
                "add": address,
                "port": inbound.port.to_string(),
                "id": id,
                "aid": number_field(client, "alterId").unwrap_or(0).to_string(),
                "scy": "auto",
                "net": network,
                "type": "none",
                "host": host,
                "path": path,
                "tls": if security == "none" { "" } else { security.as_str() },
                "sni": sni
            });

            Ok(format!(
                "vmess://{}",
                general_purpose::STANDARD.encode(vmess.to_string())
            ))
        }
        _ => bail!("link generation is not supported for protocol '{protocol}'"),
    }
}

fn common_link_params(
    client: &Value,
    stream: &Value,
    network: &str,
    security: &str,
) -> Vec<(String, String)> {
    let mut params = vec![
        ("type".to_string(), network.to_string()),
        ("security".to_string(), security.to_string()),
    ];

    if let Some(flow) = string_field(client, "flow").filter(|value| !value.is_empty()) {
        params.push(("flow".to_string(), flow));
    }
    if let Some(host) = ws_host(stream).filter(|value| !value.is_empty()) {
        params.push(("host".to_string(), host));
    }
    if let Some(path) = ws_path(stream).filter(|value| !value.is_empty()) {
        params.push(("path".to_string(), path));
    }
    if let Some(service_name) = grpc_service_name(stream).filter(|value| !value.is_empty()) {
        params.push(("serviceName".to_string(), service_name));
    }
    if let Some(sni) = tls_sni(stream).filter(|value| !value.is_empty()) {
        params.push(("sni".to_string(), sni));
    }
    if let Some(fingerprint) = tls_fingerprint(stream).filter(|value| !value.is_empty()) {
        params.push(("fp".to_string(), fingerprint));
    }
    if let Some(public_key) = reality_public_key(stream).filter(|value| !value.is_empty()) {
        params.push(("pbk".to_string(), public_key));
    }
    if let Some(short_id) = reality_short_id(stream).filter(|value| !value.is_empty()) {
        params.push(("sid".to_string(), short_id));
    }
    if let Some(spider_x) = reality_spider_x(stream).filter(|value| !value.is_empty()) {
        params.push(("spx".to_string(), spider_x));
    }

    params
}

fn build_query(params: &mut [(String, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }

    params.sort_by(|a, b| a.0.cmp(&b.0));
    format!(
        "?{}",
        params
            .iter()
            .map(|(key, value)| format!("{}={}", key, encode_component(value)))
            .collect::<Vec<_>>()
            .join("&")
    )
}

fn default_flow(inbound: &RawInbound) -> String {
    settings_clients(&inbound.settings)
        .unwrap_or_default()
        .into_iter()
        .find_map(|client| string_field(&client, "flow").filter(|s| !s.is_empty()))
        .unwrap_or_default()
}

fn find_client_value(inbound: &RawInbound, client_id: &str) -> Result<Value> {
    settings_clients(&inbound.settings)
        .unwrap_or_default()
        .into_iter()
        .find(|client| client_identifier(&inbound.protocol, client) == client_id)
        .ok_or_else(|| {
            anyhow!(
                "client '{client_id}' was not found in inbound {}",
                inbound.id
            )
        })
}

fn settings_clients(settings: &str) -> Option<Vec<Value>> {
    parse_json_object(settings)?
        .get("clients")?
        .as_array()
        .cloned()
}

fn parse_json_object(raw: &str) -> Option<Value> {
    if raw.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(raw).ok()
}

fn client_identifier(protocol: &str, client: &Value) -> String {
    match protocol.to_ascii_lowercase().as_str() {
        "trojan" => string_field(client, "password")
            .or_else(|| string_field(client, "id"))
            .or_else(|| string_field(client, "email"))
            .unwrap_or_default(),
        "shadowsocks" => string_field(client, "email")
            .or_else(|| string_field(client, "id"))
            .unwrap_or_default(),
        _ => string_field(client, "id")
            .or_else(|| string_field(client, "password"))
            .or_else(|| string_field(client, "email"))
            .unwrap_or_default(),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|item| match item {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn number_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|item| match item {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    })
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|item| match item {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => text.parse::<bool>().ok(),
        _ => None,
    })
}

fn client_status(enable: bool, total: u64, used: u64, expiry_time: i64) -> String {
    if !enable {
        return "disabled".to_string();
    }
    if expiry_time > 0 && expiry_time <= Utc::now().timestamp_millis() {
        return "expired".to_string();
    }
    if total > 0 && used >= total {
        return "limited".to_string();
    }
    "active".to_string()
}

fn format_expiry(expiry_time: i64) -> String {
    if expiry_time <= 0 {
        return "Never".to_string();
    }

    DateTime::<Utc>::from_timestamp_millis(expiry_time)
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "Invalid".to_string())
}

fn string_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }

    current.as_str().map(ToString::to_string)
}

fn array_string_path(value: &Value, path: &[&str], index: usize) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }

    current
        .as_array()
        .and_then(|items| items.get(index))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn ws_host(stream: &Value) -> Option<String> {
    string_path(stream, &["wsSettings", "headers", "Host"])
}

fn ws_path(stream: &Value) -> Option<String> {
    string_path(stream, &["wsSettings", "path"])
}

fn grpc_service_name(stream: &Value) -> Option<String> {
    string_path(stream, &["grpcSettings", "serviceName"])
}

fn tls_sni(stream: &Value) -> Option<String> {
    string_path(stream, &["tlsSettings", "serverName"])
        .or_else(|| array_string_path(stream, &["realitySettings", "serverNames"], 0))
}

fn tls_fingerprint(stream: &Value) -> Option<String> {
    string_path(stream, &["tlsSettings", "settings", "fingerprint"])
        .or_else(|| string_path(stream, &["realitySettings", "settings", "fingerprint"]))
}

fn reality_public_key(stream: &Value) -> Option<String> {
    string_path(stream, &["realitySettings", "settings", "publicKey"])
}

fn reality_short_id(stream: &Value) -> Option<String> {
    array_string_path(stream, &["realitySettings", "shortIds"], 0)
}

fn reality_spider_x(stream: &Value) -> Option<String> {
    string_path(stream, &["realitySettings", "settings", "spiderX"])
}

fn parse_envelope<T>(text: &str) -> Result<ApiEnvelope<T>>
where
    T: DeserializeOwned,
{
    serde_json::from_str::<ApiEnvelope<T>>(text)
        .with_context(|| format!("failed to parse 3x-ui response: {}", text.trim()))
}

fn needs_relogin(status: StatusCode, text: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || should_retry_login(Some(text))
}

fn should_retry_login(message: Option<&str>) -> bool {
    let Some(message) = message else {
        return false;
    };
    let normalized = message.to_ascii_lowercase();
    normalized.contains("session")
        || normalized.contains("login")
        || normalized.contains("unauthorized")
        || normalized.contains("forbidden")
}

fn normalize_panel_url(url: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("panelUrl is empty");
    }
    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let mut parsed = Url::parse(&normalized).context("panelUrl is invalid")?;
    let base_path = panel_base_path(parsed.path());
    parsed.set_path(&base_path);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn panel_base_path(path: &str) -> String {
    // Match "/panel" only as a path component: at end or followed by "/".
    let index = path
        .find("/panel/")
        .or_else(|| {
            path.rfind("/panel")
                .filter(|&i| i + "/panel".len() == path.len())
        });

    let Some(index) = index else {
        return path.trim_end_matches('/').to_string();
    };

    let base = path[..index].trim_end_matches('/');
    if base.is_empty() {
        String::new()
    } else {
        base.to_string()
    }
}

fn local_tunnel_url(original: &Url, local_port: u16) -> Result<String> {
    let mut url = original.clone();
    url.set_host(Some("127.0.0.1"))
        .map_err(|_| anyhow!("Bastion tunnel failed: invalid local panel URL"))?;
    url.set_port(Some(local_port))
        .map_err(|_| anyhow!("Bastion tunnel failed: invalid local panel port"))?;
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn host_header_value(original: &Url, host: &str, default_port: u16) -> String {
    if let Some(port) = original.port() {
        format!("{host}:{port}")
    } else if is_default_port(original.scheme(), default_port) {
        host.to_string()
    } else {
        format!("{host}:{default_port}")
    }
}

fn is_default_port(scheme: &str, port: u16) -> bool {
    matches!((scheme, port), ("http", 80) | ("https", 443))
}

fn apply_host_header(request: RequestBuilder, host_header: Option<&str>) -> RequestBuilder {
    match host_header {
        Some(host_header) => request.header(HOST, host_header),
        None => request,
    }
}

fn panel_send_error(error: reqwest::Error, uses_bastion: bool, context: &str) -> anyhow::Error {
    let message = error.to_string();
    if uses_bastion {
        if is_tls_error(&message) {
            anyhow!("Panel TLS verification failed through bastion: {message}")
        } else if context.contains("login") {
            anyhow!("3x-ui login failed through bastion: {message}")
        } else {
            anyhow!("Cannot reach 3x-ui panel through bastion: {message}")
        }
    } else {
        anyhow!("{context}: {message}")
    }
}

fn is_tls_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("invalid peer")
        || lower.contains("unknown issuer")
        || lower.contains("not valid for name")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn expand_local_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(dirs) = directories::BaseDirs::new() {
            return dirs.home_dir().join(rest);
        }
    }
    PathBuf::from(trimmed)
}

fn validate_routing_filename(remote_filename: Option<&str>, local_path: &Path) -> Result<String> {
    let filename = remote_filename
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            local_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned)
        })
        .context("routing file name is empty")?;

    if filename.len() > 128 {
        bail!("routing file name is too long");
    }

    if filename.starts_with('.') || filename.contains('/') || filename.contains('\\') {
        bail!("routing file name must be a plain file name");
    }

    if !filename
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        bail!("routing file name may only contain letters, digits, dot, dash, and underscore");
    }

    if !filename.to_ascii_lowercase().ends_with(".dat") {
        bail!("routing file must use a .dat extension");
    }

    Ok(filename)
}

fn session_cache_key(server: &ServerConfig) -> String {
    format!(
        "{}|{}|ssl:{}|{}",
        server.id,
        server.panel_url.clone().unwrap_or_default(),
        server.ssl_verify,
        bastion_fingerprint(server)
    )
}

fn panel_tunnel_key(
    server: &ServerConfig,
    base_url: &str,
    target_host: &str,
    target_port: u16,
) -> String {
    format!(
        "{}|{base_url}|{target_host}:{target_port}|{}",
        server.id,
        bastion_fingerprint(server)
    )
}

fn bastion_fingerprint(server: &ServerConfig) -> String {
    let host = server.bastion_host.as_deref().unwrap_or_default();
    let user = server
        .bastion_user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server.ssh_user.as_str());
    let port = server.bastion_port.unwrap_or(22);
    let key = server.bastion_ssh_key_path.as_deref().unwrap_or_default();
    if host.trim().is_empty() {
        "direct".to_string()
    } else {
        format!("{user}@{host}:{port}|key:{key}")
    }
}

fn keychain_account(server: &ServerConfig) -> String {
    format!(
        "{}:{}",
        server.id,
        server.panel_url.clone().unwrap_or_default()
    )
}

fn csv_cell(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn encode_component(input: &str) -> String {
    input
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_base_path_strips_panel_routes() {
        assert_eq!(
            panel_base_path("/WgM5UTSqFEueUHYPxT/panel/xray"),
            "/WgM5UTSqFEueUHYPxT"
        );
        assert_eq!(
            panel_base_path("/WgM5UTSqFEueUHYPxT/panel/api/xray"),
            "/WgM5UTSqFEueUHYPxT"
        );
        assert_eq!(panel_base_path("/panel/xray"), "");
        assert_eq!(
            panel_base_path("/WgM5UTSqFEueUHYPxT"),
            "/WgM5UTSqFEueUHYPxT"
        );
    }

    #[test]
    fn parses_xray_panel_wrapper_with_string_obj() {
        let response = json!(
            r#"{
                "xraySetting": {
                    "routing": { "rules": [] },
                    "outbounds": [{ "tag": "direct", "protocol": "freedom" }]
                },
                "inboundTags": [],
                "clientReverseTags": [],
                "outboundTestUrl": "https://example.com/ping"
            }"#
        );

        let settings = parse_xray_panel_settings(response).expect("settings should parse");

        assert_eq!(
            settings.config["outbounds"][0]["tag"].as_str(),
            Some("direct")
        );
        assert_eq!(settings.outbound_test_url, "https://example.com/ping");
    }

    #[test]
    fn parses_xray_panel_wrapper_with_string_xray_setting() {
        let response = json!({
            "xraySetting": r#"{
                "routing": { "rules": [{ "type": "field", "outboundTag": "block" }] },
                "outbounds": [{ "tag": "block", "protocol": "blackhole" }]
            }"#,
            "outboundTestUrl": ""
        });

        let settings = parse_xray_panel_settings(response).expect("settings should parse");

        assert_eq!(
            settings.config["routing"]["rules"][0]["outboundTag"].as_str(),
            Some("block")
        );
        assert_eq!(settings.outbound_test_url, DEFAULT_OUTBOUND_TEST_URL);
    }

    #[test]
    fn accepts_raw_xray_config() {
        let response = json!({
            "routing": { "rules": [] },
            "outbounds": [{ "tag": "proxy", "protocol": "socks" }]
        });

        let settings = parse_xray_panel_settings(response).expect("settings should parse");

        assert_eq!(
            settings.config["outbounds"][0]["protocol"].as_str(),
            Some("socks")
        );
    }

    #[test]
    fn pins_api_routing_rule_before_user_rules() {
        let mut config = json!({
            "routing": {
                "rules": [
                    { "type": "field", "domain": ["geosite:category-ads-all"], "outboundTag": "blocked" },
                    { "type": "field", "inboundTag": ["api"], "outboundTag": "api" }
                ]
            }
        });

        ensure_api_routing_rule(&mut config);

        assert_eq!(
            config["routing"]["rules"][0]["outboundTag"].as_str(),
            Some("api")
        );
        assert_eq!(
            config["routing"]["rules"][1]["outboundTag"].as_str(),
            Some("blocked")
        );
    }

    #[test]
    fn validates_custom_routing_dat_filename() {
        let local_path = PathBuf::from("/tmp/geosite_custom.dat");

        assert_eq!(
            validate_routing_filename(None, &local_path).expect("filename should be valid"),
            "geosite_custom.dat"
        );
        assert_eq!(
            validate_routing_filename(Some("custom-list.dat"), &local_path)
                .expect("remote filename should be valid"),
            "custom-list.dat"
        );
        assert!(validate_routing_filename(Some("../bad.dat"), &local_path).is_err());
        assert!(validate_routing_filename(Some("bad.txt"), &local_path).is_err());
    }

    #[test]
    fn extracts_csrf_token_from_login_page() {
        let html = r#"<meta name="csrf-token" content="abc123">"#;

        assert_eq!(csrf_token_from_html(html).as_deref(), Some("abc123"));
    }
}

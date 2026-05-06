use crate::{config, config::ServerConfig, keychain, ssh};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, time::Duration};
use tauri::AppHandle;
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.3x-ui";
const LEGACY_KEYCHAIN_SERVICE: &str = "vpnctrl.3x-ui";
const INBOUNDS_API: &str = "/panel/api/inbounds";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PanelCredentials {
    username: String,
    password: String,
}

#[derive(Debug)]
pub struct PanelSession {
    base_url: String,
    client: Client,
    credentials: PanelCredentials,
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
    let primary_result = keychain::delete_password(app, KEYCHAIN_SERVICE, &account).await;
    let legacy_result = keychain::delete_password(app, LEGACY_KEYCHAIN_SERVICE, &account).await;

    match (primary_result, legacy_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(primary_error), Err(_legacy_error)) => Err(primary_error),
    }
}

pub async fn login(url: &str, user: &str, pass: &str) -> Result<PanelSession> {
    let base_url = normalize_panel_url(url)?;
    let client = Client::builder()
        .cookie_store(true)
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(16))
        .build()
        .context("failed to create 3x-ui http client")?;
    let credentials = PanelCredentials {
        username: user.to_string(),
        password: pass.to_string(),
    };

    login_with_client(&client, &base_url, &credentials).await?;

    Ok(PanelSession {
        base_url,
        client,
        credentials,
    })
}

pub async fn get_inbounds(app: &AppHandle, server: &ServerConfig) -> Result<Vec<ThreeXInbound>> {
    let mut session = PanelSession::for_server(app, server).await?;
    let raw = get_raw_inbounds(&mut session).await?;
    Ok(raw.iter().map(map_inbound).collect())
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
    let total_gb = if limit_gb <= 0.0 {
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
    client["expiryTime"] = json!(base_expiry + days * 86_400_000);

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
    ssh::execute(app, server, "systemctl restart xray").await?;
    Ok(())
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
    ssh::download_file(app, server, "/usr/local/x-ui/config.json", &path).await?;
    Ok(path.display().to_string())
}

impl PanelSession {
    async fn for_server(app: &AppHandle, server: &ServerConfig) -> Result<Self> {
        let panel_url = server
            .panel_url
            .as_deref()
            .context("panelUrl is not configured for this server")?;
        let credentials = read_credentials(app, server).await?;
        login(panel_url, &credentials.username, &credentials.password).await
    }

    async fn relogin(&mut self) -> Result<()> {
        login_with_client(&self.client, &self.base_url, &self.credentials).await
    }

    async fn get_obj<T>(&mut self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_obj(Method::GET, path, None).await
    }

    async fn post_action(&mut self, path: &str, body: Option<Value>) -> Result<()> {
        for attempt in 0..=1 {
            let mut request = self.client.request(Method::POST, self.url(path));
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = request
                .send()
                .await
                .with_context(|| format!("3x-ui request failed: {path}"))?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin().await?;
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
                    self.relogin().await?;
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
            let mut request = self.client.request(method.clone(), self.url(path));
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = request
                .send()
                .await
                .with_context(|| format!("3x-ui request failed: {path}"))?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.relogin().await?;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui request failed ({status}): {}", text.trim());
            }

            let envelope = parse_envelope::<T>(&text)?;
            if envelope.success == Some(false) {
                if should_retry_login(envelope.msg.as_deref()) && attempt == 0 {
                    self.relogin().await?;
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

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

async fn login_with_client(
    client: &Client,
    base_url: &str,
    credentials: &PanelCredentials,
) -> Result<()> {
    let response = client
        .post(format!("{base_url}/login"))
        .json(&json!({
            "username": credentials.username,
            "password": credentials.password
        }))
        .send()
        .await
        .context("3x-ui login request failed")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        bail!("3x-ui login failed ({status}): {}", text.trim());
    }

    if text.trim().is_empty() {
        return Ok(());
    }

    let envelope = parse_envelope::<Value>(&text)?;
    if envelope.success == Some(false) {
        bail!(
            "3x-ui login failed: {}",
            envelope
                .msg
                .unwrap_or_else(|| "invalid credentials".to_string())
        );
    }

    Ok(())
}

async fn read_credentials(app: &AppHandle, server: &ServerConfig) -> Result<PanelCredentials> {
    let account = keychain_account(server);
    let raw = match keychain::read_password(app, KEYCHAIN_SERVICE, &account).await? {
        Some(raw) => raw,
        None => keychain::read_password(app, LEGACY_KEYCHAIN_SERVICE, &account)
            .await?
            .context("3x-ui password is not saved in Keychain")?,
    };

    if raw.starts_with('{') {
        return serde_json::from_str::<PanelCredentials>(&raw)
            .context("failed to parse 3x-ui credentials from Keychain");
    }

    Ok(PanelCredentials {
        username: server
            .panel_user
            .clone()
            .unwrap_or_else(|| "admin".to_string()),
        password: raw,
    })
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
                .map(|value| value.max(0) as u64)
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
        .find_map(|client| string_field(&client, "flow"))
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
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("https://{trimmed}"))
    }
}

fn keychain_account(server: &ServerConfig) -> String {
    format!(
        "{}:{}",
        server.id,
        server.panel_url.clone().unwrap_or_default()
    )
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

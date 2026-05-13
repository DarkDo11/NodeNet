use crate::{config, config::ServerConfig, keychain, ssh};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use directories::UserDirs;
use futures::future::join_all;
use reqwest::{Client, Method, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "nodenet.3x-ui";
const LEGACY_KEYCHAIN_SERVICE: &str = "vpnctrl.3x-ui";
const INBOUNDS_API: &str = "/panel/api/inbounds";
const XRAY_API: &str = "/panel/api/xray";
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
    base_url: String,
    client: Client,
    credentials: PanelCredentials,
    cache_key: Option<String>,
    tunnel_key: Option<String>,
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

pub async fn get_inbounds(app: &AppHandle, server: &ServerConfig) -> Result<Vec<ThreeXInbound>> {
    let mut session = PanelSession::for_server(app, server).await?;
    let raw = get_raw_inbounds(&mut session).await?;
    Ok(raw.iter().map(map_inbound).collect())
}

pub async fn get_xray_config(app: &AppHandle, server: &ServerConfig) -> Result<Value> {
    let mut session = PanelSession::for_server(app, server).await?;
    session.get_value(XRAY_API).await
}

pub async fn save_xray_config(app: &AppHandle, server: &ServerConfig, config: Value) -> Result<()> {
    let mut session = PanelSession::for_server(app, server).await?;
    session.post_action(XRAY_API, Some(config)).await
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
    app.opener()
        .reveal_item_in_dir(&path)
        .context("failed to reveal backup in Finder")?;
    Ok(path.display().to_string())
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
    base_url: String,
    client: Client,
    tunnel_key: Option<String>,
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

    if let Err(error) = login_with_client(
        &transport.client,
        &transport.base_url,
        &credentials,
        transport.uses_bastion,
    )
    .await
    {
        invalidate_panel_tunnel(transport.tunnel_key.as_deref()).await;
        return Err(error);
    }

    Ok(PanelSession {
        base_url: transport.base_url,
        client: transport.client,
        credentials,
        cache_key: Some(cache_key),
        tunnel_key: transport.tunnel_key,
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

    let mut tunnel_key = None;
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

        // Keep requests addressed to the original panel host, preserving Host,
        // TLS SNI, and certificate validation, while TCP connects to the local
        // bastion tunnel endpoint.
        builder = builder.resolve(&host, SocketAddr::from(([127, 0, 0, 1], local_port)));
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
        base_url,
        client,
        tunnel_key,
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
        login_with_client(
            &self.client,
            &self.base_url,
            &self.credentials,
            self.uses_bastion,
        )
        .await
    }

    async fn refresh_cache(&self) {
        if let Some(cache_key) = &self.cache_key {
            SESSION_CACHE
                .lock()
                .await
                .insert(cache_key.clone(), (self.clone(), Instant::now()));
        }
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

    async fn get_value(&mut self, path: &str) -> Result<Value> {
        self.request_value(Method::GET, path, None).await
    }

    async fn post_action(&mut self, path: &str, body: Option<Value>) -> Result<()> {
        for attempt in 0..=1 {
            let mut request = self.client.request(Method::POST, self.url(path));
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.invalidate_cache().await;
                self.relogin().await?;
                self.refresh_cache().await;
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
                    self.invalidate_cache().await;
                    self.relogin().await?;
                    self.refresh_cache().await;
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

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.invalidate_cache().await;
                self.relogin().await?;
                self.refresh_cache().await;
                continue;
            }

            if !status.is_success() {
                bail!("3x-ui request failed ({status}): {}", text.trim());
            }

            let envelope = parse_envelope::<T>(&text)?;
            if envelope.success == Some(false) {
                if should_retry_login(envelope.msg.as_deref()) && attempt == 0 {
                    self.invalidate_cache().await;
                    self.relogin().await?;
                    self.refresh_cache().await;
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
            let mut request = self.client.request(method.clone(), self.url(path));
            if let Some(body) = &body {
                request = request.json(body);
            }

            let response = self.send_request(request, path).await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if needs_relogin(status, &text) && attempt == 0 {
                self.invalidate_cache().await;
                self.relogin().await?;
                self.refresh_cache().await;
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
                        self.invalidate_cache().await;
                        self.relogin().await?;
                        self.refresh_cache().await;
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
        format!("{}{}", self.base_url, path)
    }

    async fn send_request(
        &self,
        request: reqwest::RequestBuilder,
        path: &str,
    ) -> Result<reqwest::Response> {
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
    base_url: &str,
    credentials: &PanelCredentials,
    uses_bastion: bool,
) -> Result<()> {
    let response = match client
        .post(format!("{base_url}/login"))
        .json(&json!({
            "username": credentials.username,
            "password": credentials.password
        }))
        .send()
        .await
    {
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
        return Ok(());
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

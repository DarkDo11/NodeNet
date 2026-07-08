use crate::{config::ServerConfig, ssh, three_x_ui};
use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashSet;
use tauri::AppHandle;

const EXPIRING_SOON_DAYS: i64 = 30;

const LIST_SCRIPT_HEADER: &str = r#"
if command -v certbot >/dev/null 2>&1; then
  echo "CERTBOT_INSTALLED=1"
else
  echo "CERTBOT_INSTALLED=0"
fi
report_cert() {
  name="$1"
  cert="$2"
  [ -f "$cert" ] || return
  case "$cert" in
    /etc/letsencrypt/*) source_tag="certbot" ;;
    /root/cert/*) source_tag="acmeSh" ;;
    *) source_tag="unknown" ;;
  esac
  # Include IP-address SANs too, not just DNS names — 3x-ui's IP-address
  # certs (issued via the x-ui CLI, see /root/cert/ip below) have no domain
  # at all, so without this the identifier acme.sh needs to renew them is
  # silently lost and the UI falls back to a useless directory-name guess.
  domains=$(openssl x509 -in "$cert" -noout -ext subjectAltName 2>/dev/null | grep -oE '(DNS|IP Address):[^,]*' | sed -E 's/^(DNS|IP Address)://' | tr '\n' ' ' | sed 's/ *$//')
  issuer=$(openssl x509 -in "$cert" -noout -issuer 2>/dev/null | sed 's/^issuer=//')
  startdate=$(openssl x509 -in "$cert" -noout -startdate 2>/dev/null | sed 's/^notBefore=//')
  enddate=$(openssl x509 -in "$cert" -noout -enddate 2>/dev/null | sed 's/^notAfter=//')
  printf 'CERT\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$domains" "$issuer" "$startdate" "$enddate" "$source_tag"
}
for d in /etc/letsencrypt/live/*/; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  [ "$name" = "README" ] && continue
  report_cert "$name" "${d}cert.pem"
done
# 3x-ui's own CLI (x-ui menu 20 -> "Get SSL (Domain)" / "Get SSL for IP
# Address") issues certs via acme.sh straight to /root/cert/<domain-or-"ip">/,
# independent of whether that cert was ever registered as the panel's active
# cert (that's an extra confirmation step in the CLI flow) — so check here
# even if the settings-table lookup below finds nothing.
for d in /root/cert/*/; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  report_cert "$name" "${d}fullchain.pem"
done
# 3x-ui v3+: the panel's own web UI cert and the subscription-link server's
# cert (Panel Settings > Panel Certificate / Subscription Certificate) live
# as plain rows in its settings table, not under any Xray inbound, so pull
# them out directly if the panel's local DB is present.
if command -v sqlite3 >/dev/null 2>&1 && [ -f /etc/x-ui/x-ui.db ]; then
  sqlite3 /etc/x-ui/x-ui.db "SELECT value FROM settings WHERE key IN ('webCertFile','subCertFile') AND value != '';" 2>/dev/null | while IFS= read -r p; do
    [ -n "$p" ] && report_cert "$(basename "$(dirname "$p")")" "$p"
  done
fi
# Panels reached only by bare IP have no domain to issue a CA cert for, so
# they're commonly reverse-proxied behind nginx with a manually-placed or
# self-signed cert instead — not managed by Certbot, not in the 3x-ui
# settings table. Pull whatever nginx is actually configured to terminate.
if [ -d /etc/nginx ]; then
  grep -rhoE '^[[:space:]]*ssl_certificate[[:space:]]+[^;]+;' /etc/nginx 2>/dev/null \
    | grep -v '^[[:space:]]*#' \
    | sed -E 's/^[[:space:]]*ssl_certificate[[:space:]]+//; s/;[[:space:]]*$//; s/^"//; s/"$//' \
    | while IFS= read -r p; do
        [ -n "$p" ] && report_cert "$(basename "$(dirname "$p")")" "$p"
      done
fi
"#;

/// `certificateFile` paths pulled straight from a 3x-ui panel's inbound
/// TLS settings can point anywhere on disk (acme.sh's own store, a custom
/// directory, etc.), not just `/etc/letsencrypt/live/`. Each gets its own
/// `report_cert` call in the same SSH round trip as the Certbot glob scan,
/// named after the certificate file's parent directory.
fn build_list_script(extra_cert_paths: &[String]) -> String {
    let mut script = LIST_SCRIPT_HEADER.to_string();
    for path in extra_cert_paths {
        let quoted_path = shell_single_quote(path);
        script.push_str(&format!(
            "report_cert \"$(basename \"$(dirname {quoted_path})\")\" {quoted_path}\n"
        ));
    }
    script
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SslCertificate {
    pub cert_name: String,
    pub domains: Vec<String>,
    pub issuer: String,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    /// "valid" | "expiring" | "expired" | "unknown" (unparseable expiry date)
    pub status: String,
    /// "certbot" | "acmeSh" | "unknown" — which client manages this cert,
    /// determined from the file path it was found at. Renewal must be
    /// dispatched to the matching client; certbot has no record of an
    /// acme.sh-issued cert (and vice versa).
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCertificates {
    pub certbot_installed: bool,
    pub certificates: Vec<SslCertificate>,
}

fn parse_openssl_date(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim().trim_end_matches("GMT").trim();
    NaiveDateTime::parse_from_str(trimmed, "%b %e %H:%M:%S %Y")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
}

fn status_for(issued_at: Option<DateTime<Utc>>, expires_at: Option<DateTime<Utc>>) -> String {
    let Some(expiry) = expires_at else {
        return "unknown".to_string();
    };
    let now = Utc::now();
    if expiry <= now {
        return "expired".to_string();
    }
    // A fixed 30-day window means nothing for short-lived profiles (e.g.
    // Let's Encrypt's ~6-day IP certificates from 3x-ui's CLI) — every one
    // of those would show "expiring" permanently, even fresh off a renewal.
    // Scale the threshold to a third of the cert's own lifetime instead
    // (mirrors Certbot's own default: renew with 30 of 90 days left), capped
    // at EXPIRING_SOON_DAYS for ordinary long-lived certs.
    let threshold = issued_at
        .map(|issued| (expiry.signed_duration_since(issued) / 3).min(Duration::days(EXPIRING_SOON_DAYS)))
        .unwrap_or_else(|| Duration::days(EXPIRING_SOON_DAYS));
    if expiry <= now + threshold {
        "expiring".to_string()
    } else {
        "valid".to_string()
    }
}

pub async fn list_certificates(app: &AppHandle, server: &ServerConfig) -> Result<ServerCertificates> {
    // Best-effort: servers without a 3x-ui panel configured (or with the
    // panel unreachable) just contribute no extra paths, they don't fail
    // the whole listing.
    let panel_cert_paths = three_x_ui::get_inbound_certificate_paths(app, server)
        .await
        .unwrap_or_default();

    let script = build_list_script(&panel_cert_paths);
    let output = ssh::execute_combined(app, server, &script, 30).await?;

    let mut certbot_installed = false;
    let mut certificates = Vec::new();

    for line in output.lines() {
        if let Some(value) = line.strip_prefix("CERTBOT_INSTALLED=") {
            certbot_installed = value.trim() == "1";
            continue;
        }

        let rest = match line.strip_prefix("CERT\t") {
            Some(value) => value,
            None => continue,
        };
        let fields: Vec<&str> = rest.split('\t').collect();
        if fields.len() != 6 {
            continue;
        }

        let cert_name = fields[0].trim().to_string();
        let mut domains: Vec<String> = fields[1]
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect();
        if domains.is_empty() {
            domains.push(cert_name.clone());
        }
        let issuer = fields[2].trim().to_string();
        let issued_at = parse_openssl_date(fields[3]);
        let expires_at = parse_openssl_date(fields[4]);
        let status = status_for(issued_at, expires_at);
        let source = match fields[5].trim() {
            "certbot" => "certbot",
            "acmeSh" => "acmeSh",
            _ => "unknown",
        }
        .to_string();

        certificates.push(SslCertificate {
            cert_name,
            domains,
            issuer,
            issued_at,
            expires_at,
            status,
            source,
        });
    }

    // The certbot glob scan and the panel's own cert paths can both point at
    // the same file (e.g. a panel inbound configured against a Certbot cert
    // already found above); keep only the first entry per unique domain set,
    // which is the certbot-sourced one since it's emitted first.
    let mut seen_domains: HashSet<Vec<String>> = HashSet::new();
    certificates.retain(|cert| {
        let mut key = cert.domains.clone();
        key.sort();
        seen_domains.insert(key)
    });

    certificates.sort_by_key(|cert| cert.expires_at);

    Ok(ServerCertificates {
        certbot_installed,
        certificates,
    })
}

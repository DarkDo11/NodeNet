use crate::{config::ServerConfig, ssh};
use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use tauri::AppHandle;

const EXPIRING_SOON_DAYS: i64 = 30;

const LIST_SCRIPT: &str = r#"
if command -v certbot >/dev/null 2>&1; then
  echo "CERTBOT_INSTALLED=1"
else
  echo "CERTBOT_INSTALLED=0"
fi
for d in /etc/letsencrypt/live/*/; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  [ "$name" = "README" ] && continue
  cert="${d}cert.pem"
  [ -f "$cert" ] || continue
  domains=$(openssl x509 -in "$cert" -noout -ext subjectAltName 2>/dev/null | grep -o 'DNS:[^,]*' | sed 's/DNS://g' | tr '\n' ' ' | sed 's/ *$//')
  issuer=$(openssl x509 -in "$cert" -noout -issuer 2>/dev/null | sed 's/^issuer=//')
  startdate=$(openssl x509 -in "$cert" -noout -startdate 2>/dev/null | sed 's/^notBefore=//')
  enddate=$(openssl x509 -in "$cert" -noout -enddate 2>/dev/null | sed 's/^notAfter=//')
  printf 'CERT\t%s\t%s\t%s\t%s\t%s\n' "$name" "$domains" "$issuer" "$startdate" "$enddate"
done
"#;

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

fn status_for(expires_at: Option<DateTime<Utc>>) -> String {
    let Some(expiry) = expires_at else {
        return "unknown".to_string();
    };
    let now = Utc::now();
    if expiry <= now {
        "expired".to_string()
    } else if expiry <= now + Duration::days(EXPIRING_SOON_DAYS) {
        "expiring".to_string()
    } else {
        "valid".to_string()
    }
}

pub async fn list_certificates(app: &AppHandle, server: &ServerConfig) -> Result<ServerCertificates> {
    let output = ssh::execute_combined(app, server, LIST_SCRIPT, 30).await?;

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
        if fields.len() != 5 {
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
        let status = status_for(expires_at);

        certificates.push(SslCertificate {
            cert_name,
            domains,
            issuer,
            issued_at,
            expires_at,
            status,
        });
    }

    certificates.sort_by_key(|cert| cert.expires_at);

    Ok(ServerCertificates {
        certbot_installed,
        certificates,
    })
}

use crate::{config::ServerConfig, ssh};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;

const METRICS_SCRIPT: &str = r#"
CPU=$(top -bn1 | awk '/Cpu\(s\)|%Cpu/ { for (i=1; i<=NF; i++) if ($i ~ /^id,?$/) { printf "%.2f", 100 - $(i-1); exit } }')
if [ -z "$CPU" ]; then CPU=0; fi
read RAM_TOTAL RAM_USED <<EOF
$(free -m | awk '/Mem:/ {print $2, $3}')
EOF
read DISK_TOTAL DISK_USED DISK_PERCENT <<EOF
$(df -h / | awk 'NR==2 {gsub("%", "", $5); print $2, $3, $5}')
EOF
LOAD_AVERAGE=$(awk '{print $1, $2, $3}' /proc/loadavg)
UPTIME_SEC=$(cut -d. -f1 /proc/uptime)
read RX_BYTES TX_BYTES <<EOF
$(awk 'NR>2 {gsub(":", "", $1); if ($1 != "lo") {rx += $2; tx += $10}} END {printf "%.0f %.0f\n", rx, tx}' /proc/net/dev)
EOF
printf 'cpu_percent=%s\n' "$CPU"
printf 'ram_total_mb=%s\n' "$RAM_TOTAL"
printf 'ram_used_mb=%s\n' "$RAM_USED"
printf 'disk_total=%s\n' "$DISK_TOTAL"
printf 'disk_used=%s\n' "$DISK_USED"
printf 'disk_percent=%s\n' "$DISK_PERCENT"
printf 'load_average=%s\n' "$LOAD_AVERAGE"
printf 'uptime_sec=%s\n' "$UPTIME_SEC"
printf 'rx_bytes=%s\n' "$RX_BYTES"
printf 'tx_bytes=%s\n' "$TX_BYTES"
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetrics {
    pub server_id: String,
    pub timestamp: DateTime<Utc>,
    pub cpu_percent: f64,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub ram_percent: f64,
    pub disk_used: String,
    pub disk_total: String,
    pub disk_percent: f64,
    pub load_average: [f64; 3],
    pub uptime_sec: u64,
    pub uptime: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub async fn collect(server: &ServerConfig) -> Result<ServerMetrics> {
    let output = ssh::execute(server, METRICS_SCRIPT).await?;
    parse_metrics(&server.id, &output)
}

fn parse_metrics(server_id: &str, output: &str) -> Result<ServerMetrics> {
    let mut values = HashMap::new();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    let cpu_percent = parse_f64(&values, "cpu_percent")?;
    let ram_total_mb = parse_u64(&values, "ram_total_mb")?;
    let ram_used_mb = parse_u64(&values, "ram_used_mb")?;
    let disk_percent = parse_f64(&values, "disk_percent")?;
    let uptime_sec = parse_u64(&values, "uptime_sec")?;
    let rx_bytes = parse_u64(&values, "rx_bytes")?;
    let tx_bytes = parse_u64(&values, "tx_bytes")?;
    let load_average = parse_load_average(
        values
            .get("load_average")
            .context("missing load_average metric")?,
    )?;
    let ram_percent = if ram_total_mb == 0 {
        0.0
    } else {
        (ram_used_mb as f64 / ram_total_mb as f64) * 100.0
    };

    Ok(ServerMetrics {
        server_id: server_id.to_string(),
        timestamp: Utc::now(),
        cpu_percent,
        ram_used_mb,
        ram_total_mb,
        ram_percent,
        disk_used: values
            .get("disk_used")
            .cloned()
            .unwrap_or_else(|| "--".to_string()),
        disk_total: values
            .get("disk_total")
            .cloned()
            .unwrap_or_else(|| "--".to_string()),
        disk_percent,
        load_average,
        uptime_sec,
        uptime: format_uptime(uptime_sec),
        rx_bytes,
        tx_bytes,
    })
}

fn parse_f64(values: &HashMap<String, String>, key: &str) -> Result<f64> {
    Ok(values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?
        .parse::<f64>()
        .with_context(|| format!("invalid {key} metric"))?)
}

fn parse_u64(values: &HashMap<String, String>, key: &str) -> Result<u64> {
    Ok(values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?
        .parse::<u64>()
        .with_context(|| format!("invalid {key} metric"))?)
}

fn parse_load_average(raw: &str) -> Result<[f64; 3]> {
    let values = raw
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid load average")?;

    Ok([
        *values.first().unwrap_or(&0.0),
        *values.get(1).unwrap_or(&0.0),
        *values.get(2).unwrap_or(&0.0),
    ])
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

use crate::{config::ServerConfig, ssh};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use tauri::AppHandle;
use tokio::{
    process::Command,
    time::{timeout, Duration},
};

const METRICS_SCRIPT: &str = r#"
read_cpu_stat() {
  awk '/^cpu / {print $2, $3, $4, $5, $6, $7, $8, $9; exit}' /proc/stat 2>/dev/null
}

CPU_SAMPLE_1=$(read_cpu_stat)
if [ -n "$CPU_SAMPLE_1" ]; then
  sleep 0.35
  CPU_SAMPLE_2=$(read_cpu_stat)
  CPU=$(awk -v a="$CPU_SAMPLE_1" -v b="$CPU_SAMPLE_2" '
    BEGIN {
      split(a, p, " ");
      split(b, c, " ");
      prev_idle = p[4] + p[5];
      curr_idle = c[4] + c[5];
      prev_total = 0;
      curr_total = 0;
      for (i = 1; i <= 8; i++) {
        prev_total += p[i];
        curr_total += c[i];
      }
      delta_total = curr_total - prev_total;
      delta_idle = curr_idle - prev_idle;
      if (delta_total <= 0) {
        usage = 0;
      } else {
        usage = 100 * (delta_total - delta_idle) / delta_total;
      }
      if (usage < 0) usage = 0;
      if (usage > 100) usage = 100;
      printf "%.1f", usage;
    }')
else
  CPU=$(top -bn1 | awk '
    /Cpu\(s\)|%Cpu|CPU/ {
      for (i = 1; i <= NF; i++) {
        token = $i;
        gsub(",", "", token);
        if (token ~ /^(id|idle)$/ && i > 1) {
          idle = $(i - 1);
          gsub(",", "", idle);
          usage = 100 - idle;
          if (usage < 0) usage = 0;
          if (usage > 100) usage = 100;
          printf "%.1f", usage;
          exit;
        }
        if (token ~ /^[0-9.]+id$/) {
          idle = token;
          sub(/id$/, "", idle);
          usage = 100 - idle;
          if (usage < 0) usage = 0;
          if (usage > 100) usage = 100;
          printf "%.1f", usage;
          exit;
        }
      }
    }')
fi
if [ -z "$CPU" ]; then CPU=0.0; fi
read RAM_TOTAL RAM_USED <<EOF
$(free -m | awk '/Mem:/ {print $2, $3}')
EOF
read DISK_TOTAL DISK_USED DISK_PERCENT <<EOF
$(df -h / | awk 'NR==2 {gsub("%", "", $5); print $2, $3, $5}')
EOF
LOAD_AVERAGE=$(awk '{print $1, $2, $3}' /proc/loadavg)
UPTIME_SEC=$(cut -d. -f1 /proc/uptime)
read RX_BYTES TX_BYTES <<EOF
$(awk 'NR>2 {
  iface = $1;
  gsub(":", "", iface);
  if (iface == "lo") next;
  fallback_rx += $2;
  fallback_tx += $10;
  if (iface ~ /^(eth|ens|enp|eno|em|p[0-9]|en[0-9]|wl|wlan|wwan|venet|bond|team|tun|tap|wg)/) {
    rx += $2;
    tx += $10;
    matched += 1;
  }
} END {
  if (matched > 0) {
    printf "%.0f %.0f\n", rx, tx;
  } else {
    printf "%.0f %.0f\n", fallback_rx, fallback_tx;
  }
}' /proc/net/dev)
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
printf 'total_rx_bytes=%s\n' "$RX_BYTES"
printf 'total_tx_bytes=%s\n' "$TX_BYTES"
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
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub total_traffic_bytes: u64,
    pub ping_ms: Option<f64>,
    pub is_online: bool,
}

pub async fn collect(app: &AppHandle, server: &ServerConfig) -> Result<ServerMetrics> {
    let (output, ping_ms) = tokio::join!(
        ssh::execute(app, server, METRICS_SCRIPT),
        ping_host(&server.host)
    );
    let mut metrics = parse_metrics(&server.id, &output?)?;
    metrics.ping_ms = ping_ms;
    metrics.is_online = true;
    Ok(metrics)
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
    let total_rx_bytes = values
        .get("total_rx_bytes")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(rx_bytes);
    let total_tx_bytes = values
        .get("total_tx_bytes")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(tx_bytes);
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
        cpu_percent: round_one(cpu_percent.clamp(0.0, 100.0)),
        ram_used_mb,
        ram_total_mb,
        ram_percent: round_one(ram_percent.clamp(0.0, 100.0)),
        disk_used: values
            .get("disk_used")
            .cloned()
            .unwrap_or_else(|| "--".to_string()),
        disk_total: values
            .get("disk_total")
            .cloned()
            .unwrap_or_else(|| "--".to_string()),
        disk_percent: round_one(disk_percent.clamp(0.0, 100.0)),
        load_average,
        uptime_sec,
        uptime: format_uptime(uptime_sec),
        rx_bytes,
        tx_bytes,
        total_rx_bytes,
        total_tx_bytes,
        total_traffic_bytes: total_rx_bytes.saturating_add(total_tx_bytes),
        ping_ms: None,
        is_online: true,
    })
}

fn parse_f64(values: &HashMap<String, String>, key: &str) -> Result<f64> {
    values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?
        .parse::<f64>()
        .with_context(|| format!("invalid {key} metric"))
}

fn parse_u64(values: &HashMap<String, String>, key: &str) -> Result<u64> {
    values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?
        .parse::<u64>()
        .with_context(|| format!("invalid {key} metric"))
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

async fn ping_host(host: &str) -> Option<f64> {
    let output = timeout(
        Duration::from_secs(3),
        Command::new("ping")
            .args(["-c", "1", "-W", "2000", host])
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_ping_ms(&combined)
}

fn parse_ping_ms(output: &str) -> Option<f64> {
    output
        .split_whitespace()
        .find_map(|part| part.strip_prefix("time="))
        .and_then(|value| value.trim_end_matches("ms").parse::<f64>().ok())
        .map(round_one)
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
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

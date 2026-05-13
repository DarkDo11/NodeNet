use crate::{config::ServerConfig, ssh};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use tauri::AppHandle;

const METRICS_SCRIPT: &str = r#"
export LC_ALL=C
export LANG=C
export LANGUAGE=C
export LC_NUMERIC=C
read RAM_TOTAL RAM_USED <<EOF
$(free -m | awk '/Mem:/ {print $2, $3}')
EOF
read DISK_TOTAL DISK_USED DISK_PERCENT <<EOF
$(df -h / | awk 'NR==2 {gsub("%", "", $5); print $2, $3, $5}')
EOF
LOAD_AVERAGE=$(awk '{print $1, $2, $3}' /proc/loadavg)
LOAD1=$(printf '%s\n' "$LOAD_AVERAGE" | awk '{print $1}')
CPU_CORES=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || awk '/^processor[[:space:]]*:/ {count++} END {print count + 0}' /proc/cpuinfo 2>/dev/null)
if [ -z "$CPU_CORES" ] || [ "$CPU_CORES" -le 0 ] 2>/dev/null; then CPU_CORES=1; fi
CPU=$(awk -v load="$LOAD1" -v cores="$CPU_CORES" '
  BEGIN {
    # Normalized CPU load: Linux 1-minute load average divided by online CPU
    # cores. 100% means load equals total core capacity; values above 100%
    # mean the run queue exceeds available cores. This is load average, not
    # /proc/stat utilization.
    if (cores <= 0 || load < 0) {
      normalized = 0;
    } else {
      normalized = (load / cores) * 100;
    }
    printf "%.1f", normalized;
  }')
if ! printf '%s\n' "$CPU" | awk '/^[0-9]+([.][0-9]+)?$/ { ok = 1 } END { exit ok ? 0 : 1 }'; then
  CPU=0
fi
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
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetrics {
    pub server_id: String,
    pub timestamp: DateTime<Utc>,
    /// Normalized CPU load %, calculated as load1 / online CPU cores * 100.
    /// This is Linux load average, not /proc/stat utilization.
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
        ssh::ping_ms(app, server)
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

    let load_average = parse_load_average(
        values
            .get("load_average")
            .context("missing load_average metric")?,
    )?;
    let cpu_cores = values
        .get("cpu_cores")
        .and_then(|value| parse_u64_value(value).ok())
        .unwrap_or(1)
        .max(1);
    let cpu_percent = parse_f64(&values, "cpu_percent")
        .unwrap_or_else(|_| (load_average[0] / cpu_cores as f64) * 100.0);
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
    let ram_percent = if ram_total_mb == 0 {
        0.0
    } else {
        (ram_used_mb as f64 / ram_total_mb as f64) * 100.0
    };

    Ok(ServerMetrics {
        server_id: server_id.to_string(),
        timestamp: Utc::now(),
        cpu_percent: round_one(cpu_percent.max(0.0)),
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
    let raw = values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?;

    parse_decimal(raw).with_context(|| format!("invalid {key} metric"))
}

fn parse_u64(values: &HashMap<String, String>, key: &str) -> Result<u64> {
    let raw = values
        .get(key)
        .with_context(|| format!("missing {key} metric"))?;

    parse_u64_value(raw).with_context(|| format!("invalid {key} metric"))
}

fn parse_u64_value(raw: &str) -> Result<u64> {
    raw.trim().parse::<u64>().map_err(Into::into)
}

fn parse_load_average(raw: &str) -> Result<[f64; 3]> {
    let values = raw
        .split_whitespace()
        .map(parse_decimal)
        .collect::<Result<Vec<_>>>()
        .context("invalid load average")?;

    Ok([
        *values.first().unwrap_or(&0.0),
        *values.get(1).unwrap_or(&0.0),
        *values.get(2).unwrap_or(&0.0),
    ])
}

fn parse_decimal(raw: &str) -> Result<f64> {
    let normalized = raw.trim().replace(',', ".");
    let value = normalized.parse::<f64>()?;

    if value.is_finite() {
        Ok(value)
    } else {
        anyhow::bail!("non-finite value")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OUTPUT: &str = r#"
cpu_percent=12.3
cpu_cores=4
ram_total_mb=2048
ram_used_mb=1024
disk_total=50G
disk_used=20G
disk_percent=40
load_average=0.12 0.34 0.56
uptime_sec=3661
rx_bytes=100
tx_bytes=200
total_rx_bytes=1000
total_tx_bytes=2000
"#;

    #[test]
    fn parses_metrics_output() {
        let metrics = parse_metrics("server-1", SAMPLE_OUTPUT).expect("metrics should parse");

        assert_eq!(metrics.server_id, "server-1");
        assert_eq!(metrics.cpu_percent, 12.3);
        assert_eq!(metrics.ram_percent, 50.0);
        assert_eq!(metrics.disk_percent, 40.0);
        assert_eq!(metrics.load_average, [0.12, 0.34, 0.56]);
        assert_eq!(metrics.total_traffic_bytes, 3000);
        assert_eq!(metrics.uptime, "1h 1m");
    }

    #[test]
    fn accepts_decimal_comma_metrics() {
        let output = SAMPLE_OUTPUT
            .replace("cpu_percent=12.3", "cpu_percent=12,3")
            .replace("disk_percent=40", "disk_percent=40,5")
            .replace("load_average=0.12 0.34 0.56", "load_average=0,12 0,34 0,56");

        let metrics = parse_metrics("server-1", &output).expect("metrics should parse");

        assert_eq!(metrics.cpu_percent, 12.3);
        assert_eq!(metrics.disk_percent, 40.5);
        assert_eq!(metrics.load_average, [0.12, 0.34, 0.56]);
    }

    #[test]
    fn falls_back_to_load_when_cpu_metric_is_non_finite() {
        let output = SAMPLE_OUTPUT.replace("cpu_percent=12.3", "cpu_percent=NaN");

        let metrics = parse_metrics("server-1", &output).expect("metrics should parse");

        assert_eq!(metrics.cpu_percent, 3.0);
    }

    #[test]
    fn falls_back_to_load_when_cpu_metric_is_blank() {
        let output = SAMPLE_OUTPUT
            .replace("cpu_percent=12.3", "cpu_percent=")
            .replace("cpu_cores=4", "cpu_cores=2")
            .replace("load_average=0.12 0.34 0.56", "load_average=1.50 0.34 0.56");

        let metrics = parse_metrics("server-1", &output).expect("metrics should parse");

        assert_eq!(metrics.cpu_percent, 75.0);
    }
}

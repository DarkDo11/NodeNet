import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import {
  Activity,
  ArrowDown,
  ArrowUp,
  Cpu,
  Database,
  ExternalLink,
  Gauge,
  HardDrive,
  MemoryStick,
  Radio,
  Timer,
} from "lucide-react";
import type { ReactNode } from "react";
import MetricChart from "./MetricChart";
import type { MetricPoint, MetricsRange, PingResult, ServerConfig, ServerMetrics } from "../types";

interface DashboardProps {
  server: ServerConfig | null;
  metrics: ServerMetrics | undefined;
  history: MetricPoint[];
  selectedRange: MetricsRange;
  uptimeSummary: {
    percent: number | null;
    offlineEvents: number;
    totalPoints: number;
  };
  status: PingResult | undefined;
  error: string | undefined;
  isPolling: boolean;
  onRangeChange: (range: MetricsRange) => void;
  onRetry: () => void;
}

const cardVariants = {
  hidden: { opacity: 0, y: 12 },
  visible: (index: number) => ({
    opacity: 1,
    y: 0,
    transition: { delay: index * 0.04, duration: 0.28 },
  }),
};

const formatPercent = (value = 0) => `${value.toFixed(1)}%`;

const formatBytes = (bytes = 0) => {
  if (bytes >= 1_000_000_000_000) return `${(bytes / 1_000_000_000_000).toFixed(1)} TB`;
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes.toFixed(0)} B`;
};

const formatRate = (bytes = 0) => {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB/s`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB/s`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB/s`;
  return `${bytes.toFixed(0)} B/s`;
};

const latestPoint = (history: MetricPoint[]) => history[history.length - 1];

const formatPing = (pingMs: number | null | undefined, isOnline: boolean | undefined) => {
  if (isOnline === false) return "Offline";
  if (typeof pingMs !== "number") return "—";
  return `${pingMs.toFixed(1)} ms`;
};

const formatPingChartValue = (value: number) =>
  `${Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1)} ms`;

const formatUptimePercent = (percent: number | null, totalPoints: number) => {
  if (percent === null || totalPoints < 3) return "--";
  return `${percent.toFixed(percent >= 99.9 ? 2 : 1)}%`;
};

const ranges: Array<{ value: MetricsRange; label: string }> = [
  { value: "all", label: "All" },
  { value: "1d", label: "1D" },
  { value: "1w", label: "1W" },
  { value: "1m", label: "1M" },
  { value: "1y", label: "1Y" },
];

function MetricCard({
  icon,
  label,
  value,
  detail,
  accent,
  index,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  detail: string;
  accent: "green" | "blue" | "yellow" | "red" | "neutral";
  index: number;
}) {
  return (
    <motion.article
      className={`metric-card ${accent}`}
      custom={index}
      initial="hidden"
      animate="visible"
      variants={cardVariants}
    >
      <div className="metric-card-top">
        <span className="metric-icon">{icon}</span>
        <span>{label}</span>
      </div>
      <strong>{value}</strong>
      <small>{detail}</small>
    </motion.article>
  );
}

function MetricSkeleton({ index }: { index: number }) {
  return (
    <motion.article
      className="metric-card skeleton-card"
      custom={index}
      initial="hidden"
      animate="visible"
      variants={cardVariants}
    >
      <span className="skeleton-line short" />
      <span className="skeleton-line tall" />
      <span className="skeleton-line" />
    </motion.article>
  );
}

export default function Dashboard({
  server,
  metrics,
  history,
  selectedRange,
  uptimeSummary,
  status,
  error,
  isPolling,
  onRangeChange,
  onRetry,
}: DashboardProps) {
  const point = latestPoint(history);
  const showSkeletons = isPolling && !metrics;

  if (!server) {
    return (
      <main className="content">
        <div className="empty-state">
          <Database size={28} />
          <h2>No server selected</h2>
        </div>
      </main>
    );
  }

  const trafficDetail = point
    ? `${formatBytes(point.totalRxBytes)} down · ${formatBytes(point.totalTxBytes)} up`
    : "No traffic sample";
  const pingMs = point?.pingMs ?? metrics?.pingMs ?? status?.latencyMs ?? null;
  const isOnline = point?.isOnline ?? (status ? status.status !== "offline" : undefined);
  const uptimeDetail = uptimeSummary.totalPoints < 3
    ? "Not enough data"
    : `${uptimeSummary.offlineEvents} offline event${uptimeSummary.offlineEvents === 1 ? "" : "s"} · ${uptimeSummary.totalPoints} pts`;

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">{server.country} / {server.id}</p>
          <h2>{server.name}</h2>
          <span className="server-target">{server.sshUser}@{server.host}:{server.sshPort}</span>
        </div>
        <div className="header-actions">
          <div className="range-selector" aria-label="Metrics range">
            {ranges.map((range) => (
              <button
                key={range.value}
                className={selectedRange === range.value ? "active" : ""}
                onClick={() => onRangeChange(range.value)}
              >
                {range.label}
              </button>
            ))}
          </div>
          <span className={`health-pill ${status?.status ?? "unknown"}`}>
            {status?.status ?? "unknown"}
            {typeof status?.latencyMs === "number" ? ` · ${status.latencyMs} ms` : ""}
          </span>
          {server.panelUrl ? (
            <button
              className="icon-button"
              onClick={() => void openUrl(server.panelUrl ?? "")}
              title="Open panel"
            >
              <ExternalLink size={17} />
            </button>
          ) : null}
        </div>
      </header>

      {error ? (
        <div className="error-state">
          <div>
            <strong>Metrics unavailable</strong>
            <span>{error}</span>
          </div>
          <button className="command-button" onClick={onRetry}>Retry</button>
        </div>
      ) : null}

      <section className="metrics-grid">
        {showSkeletons
          ? Array.from({ length: 8 }, (_, index) => <MetricSkeleton key={index} index={index} />)
          : (
            <>
              <MetricCard
                icon={<Cpu size={18} />}
                label="CPU Usage"
                value={formatPercent(metrics?.cpuPercent)}
                detail="utilization"
                accent="yellow"
                index={0}
              />
              <MetricCard
                icon={<MemoryStick size={18} />}
                label="RAM"
                value={formatPercent(metrics?.ramPercent)}
                detail={`${metrics?.ramUsedMb ?? 0} MB of ${metrics?.ramTotalMb ?? 0} MB`}
                accent="green"
                index={1}
              />
              <MetricCard
                icon={<HardDrive size={18} />}
                label="Disk"
                value={formatPercent(metrics?.diskPercent)}
                detail={`${metrics?.diskUsed ?? "--"} of ${metrics?.diskTotal ?? "--"}`}
                accent="blue"
                index={2}
              />
              <MetricCard
                icon={<Timer size={18} />}
                label="Uptime"
                value={formatUptimePercent(uptimeSummary.percent, uptimeSummary.totalPoints)}
                detail={uptimeDetail}
                accent="neutral"
                index={3}
              />
              <MetricCard
                icon={<Database size={18} />}
                label="Traffic total"
                value={formatBytes(point?.totalTrafficBytes ?? metrics?.totalTrafficBytes ?? 0)}
                detail={trafficDetail}
                accent="blue"
                index={4}
              />
              <MetricCard
                icon={<ArrowDown size={18} />}
                label="Download rate"
                value={formatRate(point?.rxRateBps)}
                detail="bytes per second"
                accent="green"
                index={5}
              />
              <MetricCard
                icon={<ArrowUp size={18} />}
                label="Upload rate"
                value={formatRate(point?.txRateBps)}
                detail="bytes per second"
                accent="red"
                index={6}
              />
              <MetricCard
                icon={<Radio size={18} />}
                label="Ping"
                value={formatPing(pingMs, isOnline)}
                detail={typeof pingMs === "number" ? "ICMP latency" : "unknown"}
                accent="yellow"
                index={7}
              />
            </>
          )}
      </section>

      <section className="charts-grid">
        <MetricChart
          title="CPU Usage / RAM / Disk"
          data={history}
          range={selectedRange}
          domain={[0, 100]}
          unitFormatter={(value) => `${value.toFixed(0)}%`}
          series={[
            { key: "cpu", name: "CPU Usage", color: "#ffcc66" },
            { key: "ram", name: "RAM", color: "#51d88a" },
            { key: "disk", name: "Disk", color: "#57b9ff" },
          ]}
        />
        <MetricChart
          title="Traffic rate"
          data={history}
          range={selectedRange}
          unitFormatter={formatRate}
          series={[
            { key: "rxRateBps", name: "Download", color: "#51d88a", type: "area" },
            { key: "txRateBps", name: "Upload", color: "#57b9ff", type: "area" },
          ]}
        />
        <MetricChart
          title="Ping"
          data={history}
          range={selectedRange}
          domain={[0, "auto"]}
          unitFormatter={formatPingChartValue}
          series={[{ key: "pingMs", name: "Ping", color: "#ffcc66" }]}
        />
      </section>

      <footer className="dashboard-footer">
        <Gauge size={15} />
        <span>Last sample {metrics ? new Date(metrics.timestamp).toLocaleString() : "--"}</span>
        <Activity size={15} />
      </footer>
    </main>
  );
}

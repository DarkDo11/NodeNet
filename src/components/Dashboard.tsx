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
  Timer,
} from "lucide-react";
import type { ReactNode } from "react";
import MetricChart from "./MetricChart";
import type { MetricPoint, PingResult, ServerConfig, ServerMetrics } from "../types";

interface DashboardProps {
  server: ServerConfig | null;
  metrics: ServerMetrics | undefined;
  history: MetricPoint[];
  status: PingResult | undefined;
  error: string | undefined;
  isPolling: boolean;
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

const formatBits = (bits = 0) => {
  if (bits >= 1_000_000_000) return `${(bits / 1_000_000_000).toFixed(1)} Gb/s`;
  if (bits >= 1_000_000) return `${(bits / 1_000_000).toFixed(1)} Mb/s`;
  if (bits >= 1_000) return `${(bits / 1_000).toFixed(1)} Kb/s`;
  return `${bits.toFixed(0)} b/s`;
};

const latestPoint = (history: MetricPoint[]) => history.at(-1);

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

export default function Dashboard({
  server,
  metrics,
  history,
  status,
  error,
  isPolling,
}: DashboardProps) {
  const point = latestPoint(history);

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
    ? `${formatBytes(metrics?.rxBytes)} down · ${formatBytes(metrics?.txBytes)} up`
    : "No traffic sample";

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">{server.country} / {server.id}</p>
          <h2>{server.name}</h2>
          <span className="server-target">{server.sshUser}@{server.host}:{server.sshPort}</span>
        </div>
        <div className="header-actions">
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

      {error ? <div className="error-banner">{error}</div> : null}

      <section className="metrics-grid">
        <MetricCard
          icon={<Cpu size={18} />}
          label="CPU"
          value={formatPercent(metrics?.cpuPercent)}
          detail={`load ${metrics?.loadAverage.map((item) => item.toFixed(2)).join(" / ") ?? "0 / 0 / 0"}`}
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
          value={metrics?.uptime ?? "--"}
          detail={isPolling ? "polling" : "idle"}
          accent="neutral"
          index={3}
        />
        <MetricCard
          icon={<ArrowDown size={18} />}
          label="Down"
          value={formatBits(point?.rxRateBps)}
          detail={trafficDetail}
          accent="green"
          index={4}
        />
        <MetricCard
          icon={<ArrowUp size={18} />}
          label="Up"
          value={formatBits(point?.txRateBps)}
          detail={trafficDetail}
          accent="red"
          index={5}
        />
      </section>

      <section className="charts-grid">
        <MetricChart title="Traffic" data={history} variant="traffic" />
        <MetricChart title="CPU history" data={history} variant="cpu" />
      </section>

      <footer className="dashboard-footer">
        <Gauge size={15} />
        <span>Last sample {metrics ? new Date(metrics.timestamp).toLocaleString() : "--"}</span>
        <Activity size={15} />
      </footer>
    </main>
  );
}

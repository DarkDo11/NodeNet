import { Activity, Gauge, HardDrive, Network, Settings, ShieldCheck, Users } from "lucide-react";
import type { MetricPoint, PingResult, ServerConfig } from "../types";

export type AppView = "dashboard" | "inbounds" | "clients" | "settings";

interface SidebarProps {
  servers: ServerConfig[];
  selectedServerId: string | null;
  statusById: Record<string, PingResult>;
  latestMetricsByServer: Record<string, MetricPoint | undefined>;
  activeView: AppView;
  onSelectServer: (serverId: string) => void;
  onChangeView: (view: AppView) => void;
}

const countryFlag = (country: string) => {
  const code = country.trim().toUpperCase();
  if (!/^[A-Z]{2}$/.test(code)) return code || "--";

  return String.fromCodePoint(
    ...code.split("").map((letter) => 127397 + letter.charCodeAt(0)),
  );
};

export default function Sidebar({
  servers,
  selectedServerId,
  statusById,
  latestMetricsByServer,
  activeView,
  onSelectServer,
  onChangeView,
}: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand-row">
        <div className="brand-mark">
          <ShieldCheck size={20} strokeWidth={2.2} />
        </div>
        <div>
          <h1>NodeNet</h1>
          <p>SSH monitor</p>
        </div>
      </div>

      <nav className="nav-tabs" aria-label="Primary">
        <button
          className={activeView === "dashboard" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("dashboard")}
          title="Dashboard"
        >
          <Gauge size={16} />
          <span>Dashboard</span>
        </button>
        <button
          className={activeView === "inbounds" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("inbounds")}
          title="Inbounds"
        >
          <Network size={16} />
          <span>Inbounds</span>
        </button>
        <button
          className={activeView === "clients" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("clients")}
          title="Clients"
        >
          <Users size={16} />
          <span>Clients</span>
        </button>
        <button
          className={activeView === "settings" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("settings")}
          title="Settings"
        >
          <Settings size={16} />
          <span>Settings</span>
        </button>
      </nav>

      <div className="server-list-header">
        <span>Servers</span>
        <Activity size={15} />
      </div>

      <div className="server-list">
        {servers.map((server) => {
          const status = statusById[server.id]?.status ?? "unknown";
          const latestMetrics = latestMetricsByServer[server.id];
          const cpuPercent = latestMetrics?.cpuPercent ?? 0;

          return (
            <button
              key={server.id}
              className={
                selectedServerId === server.id ? "server-item selected" : "server-item"
              }
              onClick={() => onSelectServer(server.id)}
            >
              <span className="server-flag">{countryFlag(server.country)}</span>
              <span className="server-main">
                <span className="server-name">{server.name}</span>
                <span className="server-host">{server.sshUser}@{server.host}</span>
                <span className="cpu-mini">
                  <span style={{ width: `${Math.min(100, cpuPercent)}%` }} />
                </span>
              </span>
              <span className={`status-dot ${status}`} />
            </button>
          );
        })}

        {servers.length === 0 ? (
          <div className="empty-server-list">
            <HardDrive size={18} />
            <span>No servers</span>
          </div>
        ) : null}
      </div>
    </aside>
  );
}

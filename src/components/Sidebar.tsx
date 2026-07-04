import {
  Activity,
  Gauge,
  HardDrive,
  Lock,
  Network,
  Route,
  ScrollText,
  Settings,
  ShieldCheck,
  SquareTerminal,
  Users,
} from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import CountryFlag from "./CountryFlag";
import type { MetricPoint, PingResult, ServerConfig } from "../types";

export type AppView =
  | "dashboard"
  | "inbounds"
  | "clients"
  | "routing"
  | "terminal"
  | "events"
  | "logs"
  | "ssl"
  | "settings";

interface SidebarProps {
  servers: ServerConfig[];
  selectedServerId: string | null;
  statusById: Record<string, PingResult>;
  latestMetricsByServer: Record<string, MetricPoint | undefined>;
  activeView: AppView;
  onSelectServer: (serverId: string) => void;
  onChangeView: (view: AppView) => void;
}

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
      <div className="brand-row" data-window-drag data-tauri-drag-region>
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
          className={activeView === "routing" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("routing")}
          title="Routing"
        >
          <Route size={16} />
          <span>Routing</span>
        </button>
        <button
          className={activeView === "terminal" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("terminal")}
          title="Terminal"
        >
          <SquareTerminal size={16} />
          <span>Terminal</span>
        </button>
        <button
          className={activeView === "events" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("events")}
          title="Events Log"
        >
          <ScrollText size={16} />
          <span>Events</span>
        </button>
        <button
          className={activeView === "logs" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("logs")}
          title="Logs"
        >
          <ScrollText size={16} />
          <span>Logs</span>
        </button>
        <button
          className={activeView === "ssl" ? "nav-tab active" : "nav-tab"}
          onClick={() => onChangeView("ssl")}
          title="SSL"
        >
          <Lock size={16} />
          <span>SSL</span>
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
        <AnimatePresence initial={false}>
          {servers.map((server) => {
          const status = statusById[server.id]?.status ?? "unknown";
          const latestMetrics = latestMetricsByServer[server.id];
          const cpuLoadPercent = latestMetrics?.cpuPercent ?? 0;
          const selected = selectedServerId === server.id;

          return (
            <motion.button
              key={server.id}
              layout
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -10 }}
              transition={{ duration: 0.22 }}
              className={selected ? "server-item selected" : "server-item"}
              onClick={() => onSelectServer(server.id)}
            >
              {selected ? (
                <motion.span className="server-selection-glow" layoutId="server-selection" />
              ) : null}
              <span className="server-flag">
                <CountryFlag country={server.country} />
              </span>
              <span className="server-main">
                <span className="server-name">{server.name}</span>
                <span className="server-host">{server.sshUser}@{server.host}</span>
                <span
                  className="cpu-mini"
                  title={`CPU Load: ${cpuLoadPercent.toFixed(1)}%. loadavg 1m / CPU cores * 100`}
                  aria-label={`CPU Load: ${cpuLoadPercent.toFixed(1)}%. loadavg 1m / CPU cores * 100`}
                >
                  <span style={{ width: `${Math.max(0, cpuLoadPercent)}%` }} />
                </span>
              </span>
              <span className={`status-dot ${status}`} />
            </motion.button>
          );
        })}
        </AnimatePresence>

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

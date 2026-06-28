import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, ScrollText } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { BastionConfig, ServerConfig } from "../types";

interface LogsViewProps {
  servers: ServerConfig[];
  bastions: BastionConfig[];
  monitorServerId: string | null;
  monitorBastionId: string | null;
  selectedServerId: string | null;
}

type LogsTargetKind = "monitor" | "bastion" | "server";
type LogsPanel = "monitor" | "server";

export default function LogsView({
  servers,
  bastions,
  monitorServerId,
  monitorBastionId,
  selectedServerId,
}: LogsViewProps) {
  const [monitorTarget, setMonitorTarget] = useState("monitor");
  const [monitorLogKind, setMonitorLogKind] = useState("monitor");
  const [serverLogKind, setServerLogKind] = useState("system");
  const [serverLogServerId, setServerLogServerId] = useState("");
  const effectiveServerId = serverLogServerId || selectedServerId || servers[0]?.id || "";

  const [monitorOutput, setMonitorOutput] = useState("");
  const [serverOutput, setServerOutput] = useState("");
  const [monitorError, setMonitorError] = useState("");
  const [serverError, setServerError] = useState("");
  const [loadingPanel, setLoadingPanel] = useState<LogsPanel | null>(null);

  const monitorTargetLabel = useMemo(() => {
    if (monitorServerId) {
      return servers.find((server) => server.id === monitorServerId)?.name ?? monitorServerId;
    }
    if (monitorBastionId) {
      return bastions.find((bastion) => bastion.id === monitorBastionId)?.name ?? monitorBastionId;
    }
    return "Not configured";
  }, [bastions, monitorBastionId, monitorServerId, servers]);

  useEffect(() => {
    setServerLogServerId(selectedServerId ?? servers[0]?.id ?? "");
  }, [selectedServerId, servers]);

  const loadLogs = async (panel: LogsPanel) => {
    const isMonitorPanel = panel === "monitor";
    const targetValue = isMonitorPanel ? monitorTarget : `server:${effectiveServerId}`;
    const [kind, id] = targetValue.split(":", 2) as [LogsTargetKind, string | undefined];
    const logKind = isMonitorPanel ? monitorLogKind : serverLogKind;

    if (!isMonitorPanel && !id) {
      setServerError("Choose a server first.");
      return;
    }

    setLoadingPanel(panel);
    if (isMonitorPanel) {
      setMonitorError("");
    } else {
      setServerError("");
    }

    try {
      const output = await invoke<string>("get_remote_logs", {
        targetKind: kind,
        targetId: id ?? null,
        logKind,
      });
      if (isMonitorPanel) {
        setMonitorOutput(output.trim() || "No logs returned");
      } else {
        setServerOutput(output.trim() || "No logs returned");
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (isMonitorPanel) {
        setMonitorError(message);
      } else {
        setServerError(message);
      }
    } finally {
      setLoadingPanel(null);
    }
  };

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">Diagnostics</p>
          <h2>Logs</h2>
          <span className="server-target">Monitor target: {monitorTargetLabel}</span>
        </div>
      </header>

      <section className="logs-grid">
        <article className="logs-panel">
          <div className="settings-panel-header split">
            <div>
              <ScrollText size={18} />
              <h3>Monitor / Bastion</h3>
            </div>
            <button className="command-button" disabled={loadingPanel === "monitor"} onClick={() => void loadLogs("monitor")}>
              <RefreshCw size={16} className={loadingPanel === "monitor" ? "spin" : ""} />
              <span>{loadingPanel === "monitor" ? "Loading" : "Refresh"}</span>
            </button>
          </div>
          <div className="logs-toolbar">
            <label className="field">
              <span>Target</span>
              <select value={monitorTarget} onChange={(event) => setMonitorTarget(event.target.value)}>
                <option value="monitor">Current monitor</option>
                {bastions.map((bastion) => (
                  <option key={bastion.id} value={`bastion:${bastion.id}`}>
                    Bastion · {bastion.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>Log</span>
              <select value={monitorLogKind} onChange={(event) => setMonitorLogKind(event.target.value)}>
                <option value="monitor">NodeNet monitor</option>
                <option value="system">System</option>
              </select>
            </label>
          </div>
          {monitorError ? <div className="error-state compact">{monitorError}</div> : null}
          <pre className="logs-output">{monitorOutput || "Choose target and refresh logs."}</pre>
        </article>

        <article className="logs-panel">
          <div className="settings-panel-header split">
            <div>
              <ScrollText size={18} />
              <h3>Servers</h3>
            </div>
            <button className="command-button" disabled={loadingPanel === "server"} onClick={() => void loadLogs("server")}>
              <RefreshCw size={16} className={loadingPanel === "server" ? "spin" : ""} />
              <span>{loadingPanel === "server" ? "Loading" : "Refresh"}</span>
            </button>
          </div>
          <div className="logs-toolbar">
            <label className="field">
              <span>Server</span>
              <select value={effectiveServerId} onChange={(event) => setServerLogServerId(event.target.value)}>
                {servers.map((server) => (
                  <option key={server.id} value={server.id}>
                    {server.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>Log</span>
              <select value={serverLogKind} onChange={(event) => setServerLogKind(event.target.value)}>
                <option value="system">System</option>
                <option value="panel">3x-ui / Xray</option>
              </select>
            </label>
          </div>
          {serverError ? <div className="error-state compact">{serverError}</div> : null}
          <pre className="logs-output">{serverOutput || "Choose server and refresh logs."}</pre>
        </article>
      </section>
    </main>
  );
}

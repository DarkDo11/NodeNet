import { RefreshCw, RotateCcw } from "lucide-react";
import type { ServerConfig, ThreeXInbound } from "../types";

interface InboundsProps {
  server: ServerConfig | null;
  inbounds: ThreeXInbound[];
  selectedInboundId: number | null;
  error?: string;
  isLoading: boolean;
  isRunningAction: boolean;
  onRefresh: () => void;
  onSelectInbound: (inboundId: number) => void;
  onRestartXray: () => void;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1_000_000_000_000) return `${(bytes / 1_000_000_000_000).toFixed(2)} TB`;
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes.toFixed(0)} B`;
};

export default function Inbounds({
  server,
  inbounds,
  selectedInboundId,
  error,
  isLoading,
  isRunningAction,
  onRefresh,
  onSelectInbound,
  onRestartXray,
}: InboundsProps) {
  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">3x-ui</p>
          <h2>Inbounds</h2>
          <span className="server-target">{server?.panelUrl ?? "panelUrl is not configured"}</span>
        </div>
        <div className="header-actions">
          <button className="command-button" disabled={!server || isLoading} onClick={onRefresh}>
            <RefreshCw size={16} className={isLoading ? "spin" : ""} />
            <span>Refresh</span>
          </button>
          <button className="command-button" disabled={!server || isRunningAction} onClick={onRestartXray}>
            <RotateCcw size={16} />
            <span>Restart Xray</span>
          </button>
        </div>
      </header>

      {error ? (
        <div className="error-state">
          <div>
            <strong>Panel unavailable</strong>
            <span>{error}</span>
          </div>
          <button className="command-button" onClick={onRefresh}>Retry</button>
        </div>
      ) : null}

      <section className="inbounds-panel">
        <div className="inbounds-table header">
          <span>Protocol</span>
          <span>Port</span>
          <span>Clients</span>
          <span>Traffic ↓</span>
          <span>Traffic ↑</span>
          <span>Status</span>
        </div>
        {isLoading && inbounds.length === 0
          ? Array.from({ length: 5 }, (_, index) => (
            <div key={index} className="inbounds-table row skeleton-row">
              <span className="skeleton-line" />
              <span className="skeleton-line" />
              <span className="skeleton-line" />
              <span className="skeleton-line" />
              <span className="skeleton-line" />
              <span className="skeleton-line" />
            </div>
          ))
          : inbounds.map((inbound) => (
            <button
              key={inbound.id}
              className={
                selectedInboundId === inbound.id ? "inbounds-table row selected" : "inbounds-table row"
              }
              onClick={() => onSelectInbound(inbound.id)}
            >
              <strong>{inbound.protocol.toUpperCase()}</strong>
              <code>{inbound.port}</code>
              <span>{inbound.clientCount}</span>
              <span>{formatBytes(inbound.down)}</span>
              <span>{formatBytes(inbound.up)}</span>
              <span className={inbound.enable ? "status-label active" : "status-label disabled"}>
                {inbound.enable ? "active" : "disabled"}
              </span>
            </button>
          ))}

        {!isLoading && inbounds.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No inbounds loaded</span>
            <button className="command-button" onClick={onRefresh}>Refresh</button>
          </div>
        ) : null}
      </section>
    </main>
  );
}

import { Plus, QrCode, RefreshCw, RotateCcw, Timer, Trash2 } from "lucide-react";
import { useState } from "react";
import type { ServerConfig, ThreeXClient, ThreeXInbound } from "../types";

interface ClientsProps {
  server: ServerConfig | null;
  inbound: ThreeXInbound | null;
  clients: ThreeXClient[];
  error?: string;
  isLoading: boolean;
  isRunningAction: boolean;
  onRefresh: () => void;
  onAddClient: (name: string, limitGb: number, expireDays: number) => void;
  onReset: (client: ThreeXClient) => void;
  onDelete: (client: ThreeXClient) => void;
  onExtend: (client: ThreeXClient, days: number) => void;
  onQr: (client: ThreeXClient) => void;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1_000_000_000_000) return `${(bytes / 1_000_000_000_000).toFixed(2)} TB`;
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes.toFixed(0)} B`;
};

export default function Clients({
  server,
  inbound,
  clients,
  error,
  isLoading,
  isRunningAction,
  onRefresh,
  onAddClient,
  onReset,
  onDelete,
  onExtend,
  onQr,
}: ClientsProps) {
  const [name, setName] = useState("");
  const [limitGb, setLimitGb] = useState(30);
  const [expireDays, setExpireDays] = useState(30);

  const canAct = Boolean(server && inbound) && !isRunningAction;

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">3x-ui</p>
          <h2>Clients</h2>
          <span className="server-target">
            {inbound ? `${inbound.remark || inbound.protocol} / ${inbound.port}` : "Select an inbound"}
          </span>
        </div>
        <button className="command-button" disabled={!inbound || isLoading} onClick={onRefresh}>
          <RefreshCw size={16} className={isLoading ? "spin" : ""} />
          <span>Refresh</span>
        </button>
      </header>

      {error ? <div className="error-banner">{error}</div> : null}

      <section className="client-toolbar">
        <label className="field">
          <span>Name</span>
          <input value={name} onChange={(event) => setName(event.target.value)} placeholder="client-email" />
        </label>
        <label className="field">
          <span>Limit, GB</span>
          <input
            type="number"
            min={0}
            value={limitGb}
            onChange={(event) => setLimitGb(Number(event.target.value))}
          />
        </label>
        <label className="field">
          <span>Expire, days</span>
          <input
            type="number"
            min={0}
            value={expireDays}
            onChange={(event) => setExpireDays(Number(event.target.value))}
          />
        </label>
        <button
          className="command-button"
          disabled={!canAct || name.trim().length === 0}
          onClick={() => {
            onAddClient(name, limitGb, expireDays);
            setName("");
          }}
        >
          <Plus size={16} />
          <span>Add</span>
        </button>
      </section>

      <section className="clients-grid">
        {clients.map((client) => {
          const used = client.up + client.down;

          return (
            <article key={client.id} className={`client-card ${client.status}`}>
              <div className="client-card-header">
                <div>
                  <strong>{client.email}</strong>
                  <span>{client.protocol.toUpperCase()} / {client.port}</span>
                </div>
                <span className={`status-label ${client.status}`}>{client.status}</span>
              </div>
              <div className="traffic-row">
                <span>↓ {formatBytes(client.down)}</span>
                <span>↑ {formatBytes(client.up)}</span>
              </div>
              <div className="limit-row">
                <span>{formatBytes(used)}</span>
                <span>{client.total > 0 ? formatBytes(client.total) : "Unlimited"}</span>
              </div>
              <div className="usage-bar">
                <span style={{ width: `${Math.min(100, client.usedPercent)}%` }} />
              </div>
              <div className="client-expiry">
                <Timer size={14} />
                <span>{client.expiry}</span>
              </div>
              <div className="client-actions">
                <button className="icon-button" disabled={!canAct} onClick={() => onReset(client)} title="Reset traffic">
                  <RotateCcw size={16} />
                </button>
                <button className="icon-button" disabled={!canAct} onClick={() => onExtend(client, 30)} title="Extend 30 days">
                  <Timer size={16} />
                </button>
                <button className="icon-button" disabled={!canAct} onClick={() => onQr(client)} title="Show QR">
                  <QrCode size={16} />
                </button>
                <button className="icon-button danger" disabled={!canAct} onClick={() => onDelete(client)} title="Delete">
                  <Trash2 size={16} />
                </button>
              </div>
            </article>
          );
        })}

        {!isLoading && clients.length === 0 ? (
          <div className="chart-empty">No clients loaded</div>
        ) : null}
      </section>
    </main>
  );
}

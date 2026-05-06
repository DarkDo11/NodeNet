import { Download, Plus, QrCode, RefreshCw, RotateCcw, Search, Timer, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
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
  onResetAllExpired: () => void;
  onDeleteAllDisabled: () => void;
  onExportCsv: () => void;
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
  onResetAllExpired,
  onDeleteAllDisabled,
  onExportCsv,
}: ClientsProps) {
  const [name, setName] = useState("");
  const [limitGb, setLimitGb] = useState(30);
  const [expireDays, setExpireDays] = useState(30);
  const [extendDays, setExtendDays] = useState(30);
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState("all");
  const [sortKey, setSortKey] = useState<"email" | "traffic" | "expiry">("email");
  const [sortDirection, setSortDirection] = useState<"asc" | "desc">("asc");

  const canAct = Boolean(server && inbound) && !isRunningAction;
  const visibleClients = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    const sorted = clients
      .filter((client) =>
        normalizedQuery.length === 0 ? true : client.email.toLowerCase().includes(normalizedQuery),
      )
      .filter((client) => statusFilter === "all" || client.status === statusFilter)
      .sort((a, b) => {
        const direction = sortDirection === "asc" ? 1 : -1;
        if (sortKey === "traffic") {
          return (a.up + a.down - (b.up + b.down)) * direction;
        }
        if (sortKey === "expiry") {
          const aExpiry = a.expiryTime <= 0 ? Number.MAX_SAFE_INTEGER : a.expiryTime;
          const bExpiry = b.expiryTime <= 0 ? Number.MAX_SAFE_INTEGER : b.expiryTime;
          return (aExpiry - bExpiry) * direction;
        }
        return a.email.localeCompare(b.email) * direction;
      });
    return sorted;
  }, [clients, query, sortDirection, sortKey, statusFilter]);

  const changeSort = (nextKey: "email" | "traffic" | "expiry") => {
    if (sortKey === nextKey) {
      setSortDirection((current) => (current === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(nextKey);
      setSortDirection("asc");
    }
  };

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

      {error ? (
        <div className="error-state">
          <div>
            <strong>Clients unavailable</strong>
            <span>{error}</span>
          </div>
          <button className="command-button" disabled={!inbound} onClick={onRefresh}>Retry</button>
        </div>
      ) : null}

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

      <section className="client-list-tools">
        <label className="field search-field">
          <span>Search</span>
          <div className="input-with-icon">
            <Search size={15} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="email" />
          </div>
        </label>
        <label className="field">
          <span>Status</span>
          <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value)}>
            <option value="all">All</option>
            <option value="active">Active</option>
            <option value="expired">Expired</option>
            <option value="limited">Limited</option>
            <option value="disabled">Disabled</option>
          </select>
        </label>
        <div className="client-sort-buttons" aria-label="Sort clients">
          <button className="command-button" onClick={() => changeSort("email")}>Email</button>
          <button className="command-button" onClick={() => changeSort("traffic")}>Traffic</button>
          <button className="command-button" onClick={() => changeSort("expiry")}>Expiry</button>
        </div>
        <span className="client-count">Showing {visibleClients.length} of {clients.length} clients</span>
        <button className="command-button" disabled={!canAct} onClick={onResetAllExpired}>
          <RotateCcw size={16} />
          <span>Reset all expired</span>
        </button>
        <button className="command-button danger" disabled={!canAct} onClick={onDeleteAllDisabled}>
          <Trash2 size={16} />
          <span>Delete all disabled</span>
        </button>
        <button className="command-button" disabled={!inbound} onClick={onExportCsv}>
          <Download size={16} />
          <span>Export CSV</span>
        </button>
      </section>

      <section className="clients-grid">
        {isLoading && clients.length === 0
          ? Array.from({ length: 6 }, (_, index) => (
            <article key={index} className="client-card skeleton-card">
              <span className="skeleton-line short" />
              <span className="skeleton-line" />
              <span className="skeleton-line tall" />
              <span className="skeleton-line" />
            </article>
          ))
          : visibleClients.map((client) => {
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
                <input
                  className="extend-days-input"
                  type="number"
                  min={1}
                  max={365}
                  value={extendDays}
                  onChange={(event) => {
                    const value = Number(event.target.value);
                    if (Number.isFinite(value)) {
                      setExtendDays(Math.min(365, Math.max(1, Math.round(value))));
                    }
                  }}
                  title="Extend days"
                />
                <button className="icon-button" disabled={!canAct} onClick={() => onExtend(client, extendDays)} title={`Extend ${extendDays} days`}>
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

        {!isLoading && clients.length > 0 && visibleClients.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No clients match the current filters</span>
          </div>
        ) : null}

        {!isLoading && clients.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No clients loaded</span>
            <button className="command-button" disabled={!inbound} onClick={onRefresh}>Refresh</button>
          </div>
        ) : null}
      </section>
    </main>
  );
}

import { RefreshCw } from "lucide-react";
import type { AlertEvent } from "../types";

interface EventsLogProps {
  events: AlertEvent[];
  error: string | null;
  onRefresh: () => void;
}

export default function EventsLog({ events, error, onRefresh }: EventsLogProps) {
  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">Alerts</p>
          <h2>Events Log</h2>
          <span className="server-target">{events.length} events in memory</span>
        </div>
        <button className="command-button" onClick={onRefresh}>
          <RefreshCw size={16} />
          <span>Refresh</span>
        </button>
      </header>

      {error ? (
        <div className="error-state">
          <div>
            <strong>Events unavailable</strong>
            <span>{error}</span>
          </div>
          <button className="command-button" onClick={onRefresh}>Retry</button>
        </div>
      ) : null}

      <section className="events-panel">
        <div className="events-table header">
          <span>Type</span>
          <span>Server</span>
          <span>Time</span>
          <span>Message</span>
        </div>
        {events.map((event) => (
          <div key={event.id} className="events-table row">
            <span className={`status-label ${event.level}`}>{event.level}</span>
            <span>{event.serverName ?? event.serverId ?? "--"}</span>
            <code>{new Date(event.timestamp).toLocaleString()}</code>
            <span>{event.message}</span>
          </div>
        ))}
        {events.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No events yet</span>
          </div>
        ) : null}
      </section>
    </main>
  );
}

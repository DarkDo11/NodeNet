import { Archive, Power, RefreshCw, RotateCcw } from "lucide-react";
import type { GlobalTrafficStats, ServerConfig } from "../types";

interface TopbarProps {
  server: ServerConfig | null;
  stats: GlobalTrafficStats;
  isRunningAction: boolean;
  message: string;
  onRestartXray: () => void;
  onReboot: () => void;
  onBackup: () => void;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1_000_000_000_000) return `${(bytes / 1_000_000_000_000).toFixed(2)} TB`;
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes.toFixed(0)} B`;
};

export default function Topbar({
  server,
  stats,
  isRunningAction,
  message,
  onRestartXray,
  onReboot,
  onBackup,
}: TopbarProps) {
  const disabled = !server || isRunningAction;

  return (
    <header className="topbar">
      <div className="global-stats">
        <span>Day ↓ {formatBytes(stats.dayDown)}</span>
        <span>Day ↑ {formatBytes(stats.dayUp)}</span>
        <span>Month ↓ {formatBytes(stats.monthDown)}</span>
        <span>Month ↑ {formatBytes(stats.monthUp)}</span>
      </div>
      <div className="quick-actions">
        {message ? <span className="action-message">{message}</span> : null}
        <button className="icon-button" disabled={disabled} onClick={onRestartXray} title="Restart Xray">
          {isRunningAction ? <RefreshCw size={17} className="spin" /> : <RotateCcw size={17} />}
        </button>
        <button className="icon-button" disabled={disabled} onClick={onReboot} title="Reboot server">
          <Power size={17} />
        </button>
        <button className="icon-button" disabled={disabled} onClick={onBackup} title="Backup config">
          <Archive size={17} />
        </button>
      </div>
    </header>
  );
}

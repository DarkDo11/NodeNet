export type ServerHealth = "unknown" | "online" | "warning" | "offline";

export interface ServerConfig {
  id: string;
  name: string;
  host: string;
  sshPort: number;
  sshUser: string;
  country: string;
  panelUrl?: string | null;
  panelUser?: string | null;
  sshKeyPath?: string | null;
  bastionHost?: string | null;
  bastionPort?: number | null;
  bastionUser?: string | null;
  bastionSshKeyPath?: string | null;
  sshKeyPassphrase?: string | null;
  sslVerify: boolean;
}

export interface BastionConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  sshKeyPath?: string | null;
}

export type AppTheme = "dark" | "purple-dark" | "green-dark" | "full-dark" | "contrast" | "system";

export interface AppConfig {
  theme: AppTheme;
  pollIntervalSec: number;
  servers: ServerConfig[];
  bastions: BastionConfig[];
  monitorServerId?: string | null;
  monitorBastionId?: string | null;
}

export interface MonitorSavedServer {
  id: string;
  name: string;
  host: string;
  sshPort: number;
  sshUser: string;
  country: string;
  panelUrl?: string | null;
  sshKeyPath?: string | null;
  hasLocalConfig: boolean;
}

export interface PingResult {
  serverId: string;
  latencyMs: number | null;
  status: ServerHealth;
  message: string;
  checkedAt: string;
}

export interface ServerMetrics {
  serverId: string;
  timestamp: string;
  // Normalized CPU load %, calculated as load1 / online CPU cores * 100.
  cpuPercent: number;
  ramUsedMb: number;
  ramTotalMb: number;
  ramPercent: number;
  diskUsed: string;
  diskTotal: string;
  diskPercent: number;
  loadAverage: [number, number, number];
  uptimeSec: number;
  uptime: string;
  /** Cumulative received bytes since boot (same as totalRxBytes). */
  rxBytes: number;
  /** Cumulative transmitted bytes since boot (same as totalTxBytes). */
  txBytes: number;
  totalRxBytes?: number;
  totalTxBytes?: number;
  totalTrafficBytes?: number;
  pingMs?: number | null;
  isOnline?: boolean;
}

export type MetricsRange = "all" | "1d" | "1w" | "1m" | "1y";

export interface MetricPoint {
  serverId: string;
  timestamp: number;
  label: string;
  // Normalized CPU load %, calculated as load1 / online CPU cores * 100.
  cpu: number;
  ram: number;
  disk: number;
  rxRateBps: number;
  txRateBps: number;
  totalRxBytes: number;
  totalTxBytes: number;
  totalTrafficBytes: number;
  pingMs: number | null;
  isOnline: boolean;
  offlineEvents?: number;
  cpuPercent: number;
  ramUsedMb: number;
  ramTotalMb: number;
  ramPercent: number;
  diskUsed: string;
  diskTotal: string;
  diskPercent: number;
  loadAverage: [number, number, number];
  uptimeSec: number;
  uptime: string;
  /** @deprecated Alias for totalRxBytes. Cumulative counter, not a per-interval delta. */
  rxBytes: number;
  /** @deprecated Alias for totalTxBytes. Cumulative counter, not a per-interval delta. */
  txBytes: number;
}

export interface ThreeXInbound {
  id: number;
  remark: string;
  protocol: string;
  port: number;
  enable: boolean;
  up: number;
  down: number;
  total: number;
  clientCount: number;
}

export type ThreeXClientStatus = "active" | "disabled" | "expired" | "limited";

export interface ThreeXClient {
  id: string;
  email: string;
  inboundId: number;
  inboundRemark: string;
  protocol: string;
  port: number;
  enable: boolean;
  status: ThreeXClientStatus;
  up: number;
  down: number;
  total: number;
  expiryTime: number;
  expiry: string;
  usedPercent: number;
}

export interface GlobalTrafficStats {
  dayUp: number;
  dayDown: number;
  monthUp: number;
  monthDown: number;
  updatedAt: string | null;
}

export interface AlertEvent {
  id: string;
  level: "info" | "warn" | "error";
  kind: string;
  serverId: string | null;
  serverName: string | null;
  message: string;
  timestamp: string;
}

export interface ToastMessage {
  id: string;
  level: "info" | "warn" | "error";
  message: string;
  action?: {
    label: string;
    onClick: () => void | Promise<void>;
  };
}

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];

export interface JsonObject {
  [key: string]: JsonValue;
}

export type XrayConfig = JsonObject;

export interface TestConnectionResult {
  ping: PingResult;
  sshOk: boolean;
  sshMessage: string;
  panelOk: boolean | null;
  panelMessage: string | null;
}

export interface PanelSetupInfo {
  port: number;
  username: string;
  password: string;
  webBasePath: string;
  source: "cli" | "sqlite" | "fallback" | "default";
}

export interface TerminalStatusEvent {
  sessionId: string;
  serverId: string;
  status: "connecting" | "connected" | "reconnecting" | "disconnected";
  message: string;
}

export interface TerminalOutputEvent {
  sessionId: string;
  serverId: string;
  data: string;
}

export interface CommandOutputEvent {
  sessionId: string;
  serverId: string;
  line: string;
  done: boolean;
}

export type SslCertificateStatus = "valid" | "expiring" | "expired" | "unknown";

export interface SslCertificate {
  certName: string;
  domains: string[];
  issuer: string;
  issuedAt: string | null;
  expiresAt: string | null;
  status: SslCertificateStatus;
}

export interface ServerCertificates {
  certbotInstalled: boolean;
  certificates: SslCertificate[];
}

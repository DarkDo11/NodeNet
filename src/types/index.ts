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
  rxBytes: number;
  txBytes: number;
}

export interface MetricPoint extends ServerMetrics {
  label: string;
  rxRateBps: number;
  txRateBps: number;
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

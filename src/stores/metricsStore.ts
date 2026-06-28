import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { MetricPoint, MetricsRange, ServerMetrics } from "../types";

interface UptimeSummary {
  percent: number | null;
  offlineEvents: number;
  totalPoints: number;
}

interface MetricsState {
  metricsByServer: Record<string, ServerMetrics>;
  historyByServer: Record<string, MetricPoint[]>;
  selectedRange: MetricsRange;
  pollIntervalSec: number;
  pollingServers: Set<string>;
  errorByServer: Record<string, string>;
  offlineStrikesByServer: Record<string, number>;
  setPollInterval: (seconds: number) => void;
  setSelectedRange: (range: MetricsRange) => void;
  loadMetricsCache: () => Promise<void>;
  fetchMetrics: (serverId: string) => Promise<void>;
  isPollingServer: (serverId: string) => boolean;
  clearMetricsError: (serverId: string) => void;
  getMetricsForRange: (serverId: string, range?: MetricsRange) => MetricPoint[];
  getUptimeForRange: (serverId: string, range?: MetricsRange) => UptimeSummary;
  getCurrentPing: (serverId: string) => number | null;
}

const DAY_MS = 86_400_000;
const MAX_HISTORY_MS = 2 * 365 * DAY_MS;
const RANGE_MS: Record<Exclude<MetricsRange, "all">, number> = {
  "1d": DAY_MS,
  "1w": 7 * DAY_MS,
  "1m": 30 * DAY_MS,
  "1y": 365 * DAY_MS,
};
const MAX_RENDER_POINTS = 800;
const RANGE_STORAGE_KEY = "nodenet:metrics-range";

const numberOr = (value: unknown, fallback: unknown = 0) => {
  const number = typeof value === "number" ? value : Number(value);
  if (Number.isFinite(number)) return number;

  const fallbackNumber = typeof fallback === "number" ? fallback : Number(fallback);
  return Number.isFinite(fallbackNumber) ? fallbackNumber : 0;
};

const parseTimestamp = (value: unknown) => {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value < 10_000_000_000 ? value * 1000 : value;
  }

  if (typeof value === "string") {
    const parsed = new Date(value).getTime();
    return Number.isFinite(parsed) ? parsed : Date.now();
  }

  return Date.now();
};

const formatLabel = (timestamp: number) =>
  new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

const roundOne = (value: number) => Math.round(value * 10) / 10;

const readCpuLoadPercent = (item: Record<string, unknown>) =>
  roundOne(
    numberOr(
      item.cpu,
      numberOr(item.cpuPercent, numberOr(item.cpuLoad, item.cpu_load)),
    ),
  );

const buildPoint = (
  metrics: ServerMetrics,
  previousPoint?: MetricPoint,
): MetricPoint => {
  const timestamp = parseTimestamp(metrics.timestamp);
  const totalRxBytes = numberOr(metrics.totalRxBytes, metrics.rxBytes);
  const totalTxBytes = numberOr(metrics.totalTxBytes, metrics.txBytes);
  const elapsedSeconds = previousPoint
    ? Math.max(1, (timestamp - previousPoint.timestamp) / 1000)
    : 0;

  const rxRateBps =
    previousPoint?.isOnline && totalRxBytes >= previousPoint.totalRxBytes
      ? (totalRxBytes - previousPoint.totalRxBytes) / elapsedSeconds
      : 0;
  const txRateBps =
    previousPoint?.isOnline && totalTxBytes >= previousPoint.totalTxBytes
      ? (totalTxBytes - previousPoint.totalTxBytes) / elapsedSeconds
      : 0;

  return {
    serverId: metrics.serverId,
    timestamp,
    label: formatLabel(timestamp),
    cpu: roundOne(numberOr(metrics.cpuPercent)),
    ram: roundOne(numberOr(metrics.ramPercent)),
    disk: roundOne(numberOr(metrics.diskPercent)),
    rxRateBps,
    txRateBps,
    totalRxBytes,
    totalTxBytes,
    totalTrafficBytes: numberOr(metrics.totalTrafficBytes, totalRxBytes + totalTxBytes),
    pingMs:
      typeof metrics.pingMs === "number" && Number.isFinite(metrics.pingMs)
        ? roundOne(metrics.pingMs)
        : null,
    isOnline: metrics.isOnline ?? true,
    cpuPercent: roundOne(numberOr(metrics.cpuPercent)),
    ramUsedMb: numberOr(metrics.ramUsedMb),
    ramTotalMb: numberOr(metrics.ramTotalMb),
    ramPercent: roundOne(numberOr(metrics.ramPercent)),
    diskUsed: metrics.diskUsed ?? "--",
    diskTotal: metrics.diskTotal ?? "--",
    diskPercent: roundOne(numberOr(metrics.diskPercent)),
    loadAverage: metrics.loadAverage ?? [0, 0, 0],
    uptimeSec: numberOr(metrics.uptimeSec),
    uptime: metrics.uptime ?? "--",
    rxBytes: totalRxBytes,
    txBytes: totalTxBytes,
  };
};

const buildOfflinePoint = (serverId: string, previousPoint?: MetricPoint): MetricPoint => {
  const timestamp = Date.now();
  const totalRxBytes = previousPoint?.totalRxBytes ?? 0;
  const totalTxBytes = previousPoint?.totalTxBytes ?? 0;

  return {
    serverId,
    timestamp,
    label: formatLabel(timestamp),
    cpu: 0,
    ram: 0,
    disk: 0,
    rxRateBps: 0,
    txRateBps: 0,
    totalRxBytes,
    totalTxBytes,
    totalTrafficBytes: totalRxBytes + totalTxBytes,
    pingMs: null,
    isOnline: false,
    cpuPercent: 0,
    ramUsedMb: 0,
    ramTotalMb: 0,
    ramPercent: 0,
    diskUsed: previousPoint?.diskUsed ?? "--",
    diskTotal: previousPoint?.diskTotal ?? "--",
    diskPercent: 0,
    loadAverage: [0, 0, 0],
    uptimeSec: 0,
    uptime: "Offline",
    rxBytes: totalRxBytes,
    txBytes: totalTxBytes,
  };
};

const normalizePoint = (serverId: string, raw: unknown): MetricPoint | null => {
  if (!raw || typeof raw !== "object") return null;
  const item = raw as Record<string, unknown>;
  const timestamp = parseTimestamp(item.timestamp);
  const cpu = readCpuLoadPercent(item);
  const ram = roundOne(numberOr(item.ram, item.ramPercent));
  const disk = roundOne(numberOr(item.disk, item.diskPercent));
  const totalRxBytes = numberOr(item.totalRxBytes, item.rxBytes);
  const totalTxBytes = numberOr(item.totalTxBytes, item.txBytes);
  const pingMs = typeof item.pingMs === "number" && Number.isFinite(item.pingMs)
    ? roundOne(item.pingMs)
    : null;

  const point: MetricPoint = {
    serverId: String(item.serverId ?? serverId),
    timestamp,
    label: typeof item.label === "string" ? item.label : formatLabel(timestamp),
    cpu,
    ram,
    disk,
    rxRateBps: 0,
    txRateBps: 0,
    totalRxBytes,
    totalTxBytes,
    totalTrafficBytes: numberOr(item.totalTrafficBytes, totalRxBytes + totalTxBytes),
    pingMs,
    isOnline: typeof item.isOnline === "boolean" ? item.isOnline : true,
    cpuPercent: cpu,
    ramUsedMb: numberOr(item.ramUsedMb),
    ramTotalMb: numberOr(item.ramTotalMb),
    ramPercent: ram,
    diskUsed: typeof item.diskUsed === "string" ? item.diskUsed : "--",
    diskTotal: typeof item.diskTotal === "string" ? item.diskTotal : "--",
    diskPercent: disk,
    loadAverage: Array.isArray(item.loadAverage)
      ? [
          numberOr(item.loadAverage[0]),
          numberOr(item.loadAverage[1]),
          numberOr(item.loadAverage[2]),
        ]
      : [0, 0, 0],
    uptimeSec: numberOr(item.uptimeSec),
    uptime: typeof item.uptime === "string" ? item.uptime : "--",
    rxBytes: totalRxBytes,
    txBytes: totalTxBytes,
  };

  if (typeof item.offlineEvents === "number") {
    point.offlineEvents = item.offlineEvents;
  }

  return point;
};

const normalizeHistory = (serverId: string, rawHistory: unknown): MetricPoint[] => {
  if (!Array.isArray(rawHistory)) return [];

  const points = rawHistory
    .map((point) => normalizePoint(serverId, point))
    .filter((point): point is MetricPoint => point !== null)
    .sort((a, b) => a.timestamp - b.timestamp);

  return recomputeRates(points);
};

const recomputeRates = (points: MetricPoint[]) =>
  points.map((point, index) => {
    const previous = points[index - 1];
    if (!previous?.isOnline || !point.isOnline) {
      return { ...point, rxRateBps: 0, txRateBps: 0 };
    }

    const elapsedSeconds = Math.max(1, (point.timestamp - previous.timestamp) / 1000);
    const rxRateBps =
      point.totalRxBytes >= previous.totalRxBytes
        ? (point.totalRxBytes - previous.totalRxBytes) / elapsedSeconds
        : 0;
    const txRateBps =
      point.totalTxBytes >= previous.totalTxBytes
        ? (point.totalTxBytes - previous.totalTxBytes) / elapsedSeconds
        : 0;

    return { ...point, rxRateBps, txRateBps };
  });

const aggregatePoints = (points: MetricPoint[], bucketMs: number): MetricPoint[] => {
  const buckets = new Map<number, MetricPoint[]>();
  for (const point of points) {
    const bucket = Math.floor(point.timestamp / bucketMs) * bucketMs;
    buckets.set(bucket, [...(buckets.get(bucket) ?? []), point]);
  }

  return Array.from(buckets.entries())
    .sort(([a], [b]) => a - b)
    .map(([timestamp, bucketPoints]) => {
      const onlinePoints = bucketPoints.filter((point) => point.isOnline);
      const numericAverage = (selector: (point: MetricPoint) => number) => {
        const pts = onlinePoints.length > 0 ? onlinePoints : bucketPoints;
        return roundOne(pts.reduce((sum, point) => sum + selector(point), 0) / pts.length);
      };
      const pingValues = bucketPoints
        .map((point) => point.pingMs)
        .filter((value): value is number => typeof value === "number");
      const last = bucketPoints[bucketPoints.length - 1];
      const isOnline = onlinePoints.length / bucketPoints.length > 0.5;
      const offlineEvents = countOfflineEvents(bucketPoints);
      const pingMs =
        pingValues.length > 0
          ? roundOne(pingValues.reduce((sum, value) => sum + value, 0) / pingValues.length)
          : null;

      return {
        ...last,
        timestamp,
        label: formatLabel(timestamp),
        cpu: numericAverage((point) => point.cpu),
        ram: numericAverage((point) => point.ram),
        disk: numericAverage((point) => point.disk),
        cpuPercent: numericAverage((point) => point.cpuPercent),
        ramPercent: numericAverage((point) => point.ramPercent),
        diskPercent: numericAverage((point) => point.diskPercent),
        rxRateBps: numericAverage((point) => point.rxRateBps),
        txRateBps: numericAverage((point) => point.txRateBps),
        totalRxBytes: last.totalRxBytes,
        totalTxBytes: last.totalTxBytes,
        totalTrafficBytes: last.totalRxBytes + last.totalTxBytes,
        pingMs,
        isOnline,
        offlineEvents,
        rxBytes: last.totalRxBytes,
        txBytes: last.totalTxBytes,
      };
    });
};

const applyRetention = (history: MetricPoint[]) => {
  const now = Date.now();
  const raw = history.filter((point) => now - point.timestamp <= 7 * DAY_MS);
  const fiveMinute = aggregatePoints(
    history.filter((point) => now - point.timestamp > 7 * DAY_MS && now - point.timestamp <= 30 * DAY_MS),
    5 * 60_000,
  );
  const hourly = aggregatePoints(
    history.filter((point) => now - point.timestamp > 30 * DAY_MS && now - point.timestamp <= 365 * DAY_MS),
    60 * 60_000,
  );
  const daily = aggregatePoints(
    history.filter((point) => {
      const age = now - point.timestamp;
      return age > 365 * DAY_MS && age <= MAX_HISTORY_MS;
    }),
    DAY_MS,
  );

  return [...daily, ...hourly, ...fiveMinute, ...raw].sort((a, b) => a.timestamp - b.timestamp);
};

const downsampleForRender = (points: MetricPoint[]) => {
  if (points.length <= MAX_RENDER_POINTS) return points;

  const first = points[0];
  const last = points[points.length - 1];
  const span = Math.max(1, last.timestamp - first.timestamp);
  const bucketMs = Math.max(60_000, Math.ceil(span / MAX_RENDER_POINTS));
  return aggregatePoints(points, bucketMs);
};

const filterRange = (history: MetricPoint[], range: MetricsRange) => {
  if (range === "all") return history;
  const since = Date.now() - RANGE_MS[range];
  return history.filter((point) => point.timestamp >= since);
};

const countOfflineEvents = (points: MetricPoint[]) =>
  points.reduce((count, point, index) => {
    const previous = points[index - 1];
    return count + (!point.isOnline && previous?.isOnline !== false ? 1 : 0);
  }, 0);

const summarizeUptime = (points: MetricPoint[]): UptimeSummary => {
  if (points.length === 0) {
    return { percent: null, offlineEvents: 0, totalPoints: 0 };
  }

  const onlinePoints = points.filter((point) => point.isOnline).length;
  const storedOfflineEvents = points.reduce((sum, point) => sum + (point.offlineEvents ?? 0), 0);
  const rawPoints = points.filter((point) => point.offlineEvents === undefined);
  return {
    percent: (onlinePoints / points.length) * 100,
    offlineEvents: storedOfflineEvents + countOfflineEvents(rawPoints),
    totalPoints: points.length,
  };
};

const pointToMetrics = (point: MetricPoint): ServerMetrics => ({
  serverId: point.serverId,
  timestamp: new Date(point.timestamp).toISOString(),
  cpuPercent: point.cpuPercent,
  ramUsedMb: point.ramUsedMb,
  ramTotalMb: point.ramTotalMb,
  ramPercent: point.ramPercent,
  diskUsed: point.diskUsed,
  diskTotal: point.diskTotal,
  diskPercent: point.diskPercent,
  loadAverage: point.loadAverage,
  uptimeSec: point.uptimeSec,
  uptime: point.uptime,
  rxBytes: point.totalRxBytes,
  txBytes: point.totalTxBytes,
  totalRxBytes: point.totalRxBytes,
  totalTxBytes: point.totalTxBytes,
  totalTrafficBytes: point.totalTrafficBytes,
  pingMs: point.pingMs,
  isOnline: point.isOnline,
});

const selectedRangeFromStorage = (): MetricsRange => {
  const raw = window.localStorage.getItem(RANGE_STORAGE_KEY);
  return raw === "all" || raw === "1d" || raw === "1w" || raw === "1m" || raw === "1y"
    ? raw
    : "1d";
};

export const useMetricsStore = create<MetricsState>((set, get) => ({
  metricsByServer: {},
  historyByServer: {},
  selectedRange: selectedRangeFromStorage(),
  pollIntervalSec: 10,
  pollingServers: new Set<string>(),
  errorByServer: {},
  offlineStrikesByServer: {},

  setPollInterval: (seconds) =>
    set({ pollIntervalSec: Math.max(2, Math.round(seconds)) }),

  setSelectedRange: (range) => {
    window.localStorage.setItem(RANGE_STORAGE_KEY, range);
    set({ selectedRange: range });
  },

  loadMetricsCache: async () => {
    try {
      const rawCache = await invoke<Record<string, unknown>>("load_metrics_cache");
      const historyByServer = Object.fromEntries(
        Object.entries(rawCache).map(([serverId, history]) => [
          serverId,
          applyRetention(normalizeHistory(serverId, history)),
        ]),
      );
      const metricsByServer = Object.fromEntries(
        Object.entries(historyByServer).flatMap(([serverId, history]) => {
          const latest = history[history.length - 1];
          return latest ? [[serverId, pointToMetrics(latest)]] : [];
        }),
      ) as Record<string, ServerMetrics>;
      set({ historyByServer, metricsByServer });
    } catch {
      // Keep the last known cache when the remote monitor is briefly unreachable.
    }
  },

  fetchMetrics: async (serverId) => {
    if (get().pollingServers.has(serverId)) {
      return;
    }

    set((state) => {
      const pollingServers = new Set(state.pollingServers);
      pollingServers.add(serverId);
      return { pollingServers };
    });

    try {
      const metrics = await invoke<ServerMetrics>("get_metrics", { serverId });
      const previousHistory = get().historyByServer[serverId] ?? [];
      const previousPoint = previousHistory[previousHistory.length - 1];
      const offlineStrikes = get().offlineStrikesByServer[serverId] ?? 0;
      if (metrics.isOnline === false && previousPoint?.isOnline && offlineStrikes < 1) {
        set((state) => ({
          errorByServer: {
            ...state.errorByServer,
            [serverId]: "",
          },
          offlineStrikesByServer: {
            ...state.offlineStrikesByServer,
            [serverId]: offlineStrikes + 1,
          },
        }));
        return;
      }
      const point = buildPoint(metrics, previousPoint);
      if (previousPoint && point.timestamp <= previousPoint.timestamp) {
        return;
      }
      const history = applyRetention([...previousHistory, point]);

      set((state) => ({
        metricsByServer: {
          ...state.metricsByServer,
          [serverId]: metrics,
        },
        historyByServer: {
          ...state.historyByServer,
          [serverId]: history,
        },
        errorByServer: {
          ...state.errorByServer,
          [serverId]: "",
        },
        offlineStrikesByServer: {
          ...state.offlineStrikesByServer,
          [serverId]: metrics.isOnline === false ? offlineStrikes + 1 : 0,
        },
      }));
      void invoke("save_metrics_cache", { cache: trimCache(get().historyByServer) });
    } catch (error) {
      const previousHistory = get().historyByServer[serverId] ?? [];
      const previousPoint = previousHistory[previousHistory.length - 1];
      const offlineStrikes = get().offlineStrikesByServer[serverId] ?? 0;
      if (previousPoint?.isOnline && offlineStrikes < 1) {
        set((state) => ({
          errorByServer: {
            ...state.errorByServer,
            [serverId]: "",
          },
          offlineStrikesByServer: {
            ...state.offlineStrikesByServer,
            [serverId]: offlineStrikes + 1,
          },
        }));
        return;
      }
      const offlinePoint = buildOfflinePoint(serverId, previousPoint);
      if (previousPoint && offlinePoint.timestamp <= previousPoint.timestamp) {
        return;
      }
      const history = applyRetention([...previousHistory, offlinePoint]);
      const historyByServer = {
        ...get().historyByServer,
        [serverId]: history,
      };

      set((state) => ({
        metricsByServer: {
          ...state.metricsByServer,
          [serverId]: pointToMetrics(offlinePoint),
        },
        historyByServer,
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        offlineStrikesByServer: {
          ...state.offlineStrikesByServer,
          [serverId]: offlineStrikes + 1,
        },
      }));
      void invoke("save_metrics_cache", { cache: trimCache(historyByServer) });
    } finally {
      // Single cleanup point: handles all return paths including stale-timestamp early returns.
      set((state) => ({
        pollingServers: withoutServer(state.pollingServers, serverId),
      }));
    }
  },

  isPollingServer: (serverId) => get().pollingServers.has(serverId),

  clearMetricsError: (serverId) =>
    set((state) => ({
      errorByServer: {
        ...state.errorByServer,
        [serverId]: "",
      },
    })),

  getMetricsForRange: (serverId, range) => {
    const selectedRange = range ?? get().selectedRange;
    return downsampleForRender(filterRange(get().historyByServer[serverId] ?? [], selectedRange));
  },

  getUptimeForRange: (serverId, range) => {
    const selectedRange = range ?? get().selectedRange;
    return summarizeUptime(filterRange(get().historyByServer[serverId] ?? [], selectedRange));
  },

  getCurrentPing: (serverId) => {
    const history = get().historyByServer[serverId] ?? [];
    const latest = history[history.length - 1];
    return latest?.pingMs ?? null;
  },
}));

const withoutServer = (current: Set<string>, serverId: string) => {
  const next = new Set(current);
  next.delete(serverId);
  return next;
};

const trimCache = (cache: Record<string, MetricPoint[]>) =>
  Object.fromEntries(
    Object.entries(cache).map(([serverId, history]) => [serverId, applyRetention(history)]),
  );

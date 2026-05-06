import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { MetricPoint, ServerMetrics } from "../types";

interface MetricsState {
  metricsByServer: Record<string, ServerMetrics>;
  historyByServer: Record<string, MetricPoint[]>;
  pollIntervalSec: number;
  pollingServers: Set<string>;
  errorByServer: Record<string, string>;
  setPollInterval: (seconds: number) => void;
  loadMetricsCache: () => Promise<void>;
  fetchMetrics: (serverId: string) => Promise<void>;
  isPollingServer: (serverId: string) => boolean;
  clearMetricsError: (serverId: string) => void;
}

const MAX_POINTS = 60;

const buildPoint = (
  metrics: ServerMetrics,
  previousPoint?: MetricPoint,
): MetricPoint => {
  const timestamp = new Date(metrics.timestamp);
  const previousTimestamp = previousPoint
    ? new Date(previousPoint.timestamp)
    : null;
  const elapsedSeconds =
    previousTimestamp === null
      ? 0
      : Math.max(1, (timestamp.getTime() - previousTimestamp.getTime()) / 1000);

  const rxRateBps =
    previousPoint && metrics.rxBytes >= previousPoint.rxBytes
      ? ((metrics.rxBytes - previousPoint.rxBytes) * 8) / elapsedSeconds
      : 0;
  const txRateBps =
    previousPoint && metrics.txBytes >= previousPoint.txBytes
      ? ((metrics.txBytes - previousPoint.txBytes) * 8) / elapsedSeconds
      : 0;

  return {
    ...metrics,
    label: timestamp.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }),
    rxRateBps,
    txRateBps,
  };
};

export const useMetricsStore = create<MetricsState>((set, get) => ({
  metricsByServer: {},
  historyByServer: {},
  pollIntervalSec: 10,
  pollingServers: new Set<string>(),
  errorByServer: {},

  setPollInterval: (seconds) =>
    set({ pollIntervalSec: Math.max(2, Math.round(seconds)) }),

  loadMetricsCache: async () => {
    try {
      const historyByServer = await invoke<Record<string, MetricPoint[]>>("load_metrics_cache");
      const metricsByServer = Object.fromEntries(
        Object.entries(historyByServer).flatMap(([serverId, history]) => {
          const latest = history[history.length - 1];
          return latest ? [[serverId, latest]] : [];
        }),
      ) as Record<string, ServerMetrics>;
      set({ historyByServer, metricsByServer });
    } catch {
      set({ historyByServer: {} });
    }
  },

  fetchMetrics: async (serverId) => {
    set((state) => {
      const pollingServers = new Set(state.pollingServers);
      pollingServers.add(serverId);
      return { pollingServers };
    });

    try {
      const metrics = await invoke<ServerMetrics>("get_metrics", { serverId });
      const previousHistory = get().historyByServer[serverId] ?? [];
      const point = buildPoint(metrics, previousHistory[previousHistory.length - 1]);
      const history = [...previousHistory, point].slice(-MAX_POINTS);
      const historyByServer = {
        ...get().historyByServer,
        [serverId]: history,
      };

      set((state) => ({
        metricsByServer: {
          ...state.metricsByServer,
          [serverId]: metrics,
        },
        historyByServer,
        errorByServer: {
          ...state.errorByServer,
          [serverId]: "",
        },
        pollingServers: withoutServer(state.pollingServers, serverId),
      }));
      void invoke("save_metrics_cache", { cache: trimCache(historyByServer) });
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
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
}));

const withoutServer = (current: Set<string>, serverId: string) => {
  const next = new Set(current);
  next.delete(serverId);
  return next;
};

const trimCache = (cache: Record<string, MetricPoint[]>) =>
  Object.fromEntries(
    Object.entries(cache).map(([serverId, history]) => [serverId, history.slice(-MAX_POINTS)]),
  );

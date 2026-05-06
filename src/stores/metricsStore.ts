import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { MetricPoint, ServerMetrics } from "../types";

interface MetricsState {
  metricsByServer: Record<string, ServerMetrics>;
  historyByServer: Record<string, MetricPoint[]>;
  pollIntervalSec: number;
  isPolling: boolean;
  errorByServer: Record<string, string>;
  setPollInterval: (seconds: number) => void;
  fetchMetrics: (serverId: string) => Promise<void>;
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
  isPolling: false,
  errorByServer: {},

  setPollInterval: (seconds) =>
    set({ pollIntervalSec: Math.max(2, Math.round(seconds)) }),

  fetchMetrics: async (serverId) => {
    set({ isPolling: true });

    try {
      const metrics = await invoke<ServerMetrics>("get_metrics", { serverId });
      const previousHistory = get().historyByServer[serverId] ?? [];
      const point = buildPoint(metrics, previousHistory.at(-1));
      const history = [...previousHistory, point].slice(-MAX_POINTS);

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
        isPolling: false,
      }));
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        isPolling: false,
      }));
    }
  },

  clearMetricsError: (serverId) =>
    set((state) => ({
      errorByServer: {
        ...state.errorByServer,
        [serverId]: "",
      },
    })),
}));

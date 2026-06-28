import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { AppConfig, AppTheme, BastionConfig, PingResult, ServerConfig } from "../types";

interface ServerState {
  servers: ServerConfig[];
  bastions: BastionConfig[];
  monitorServerId: string | null;
  monitorBastionId: string | null;
  selectedServerId: string | null;
  statusById: Record<string, PingResult>;
  configPollIntervalSec: number;
  theme: AppTheme;
  isLoading: boolean;
  error: string | null;
  loadServers: () => Promise<void>;
  selectServer: (serverId: string) => void;
  upsertServer: (server: ServerConfig) => Promise<void>;
  deleteServer: (serverId: string) => Promise<void>;
  upsertBastion: (bastion: BastionConfig) => Promise<void>;
  deleteBastion: (bastionId: string) => Promise<void>;
  savePollInterval: (seconds: number) => Promise<void>;
  saveTheme: (theme: AppTheme) => Promise<void>;
  saveMonitorTarget: (target: { serverId: string | null; bastionId: string | null }) => Promise<void>;
  pingServer: (serverId: string) => Promise<void>;
  pingAllServers: () => Promise<void>;
  _pingAllInFlight: boolean;
}

const applyConfig = (
  config: AppConfig,
  currentSelected: string | null,
) => ({
  servers: config.servers,
  bastions: config.bastions ?? [],
  monitorServerId: config.monitorServerId ?? null,
  monitorBastionId: config.monitorBastionId ?? null,
  configPollIntervalSec: config.pollIntervalSec,
  theme: config.theme,
  selectedServerId:
    currentSelected && config.servers.some((server) => server.id === currentSelected)
      ? currentSelected
      : config.servers[0]?.id ?? null,
});

const softenTransientOffline = (
  result: PingResult,
  previous: PingResult | undefined,
): PingResult => {
  if (result.status !== "offline") return result;
  if (previous?.message.startsWith("Check failed once")) {
    return {
      ...result,
      message: `Repeated check failed: ${result.message}`,
    };
  }
  if (!previous || previous.status === "offline" || previous.message.startsWith("Repeated check failed")) {
    return {
      ...result,
      message: result.message,
    };
  }

  if (previous.status === "online" || previous.status === "warning") {
    return {
      ...result,
      status: "warning",
      message: `Check failed once: ${result.message}`,
    };
  }

  return result;
};

export const useServerStore = create<ServerState>((set, get) => ({
  servers: [],
  bastions: [],
  monitorServerId: null,
  monitorBastionId: null,
  selectedServerId: null,
  statusById: {},
  configPollIntervalSec: 10,
  theme: "dark",
  isLoading: true,
  error: null,
  _pingAllInFlight: false,

  loadServers: async () => {
    set({ isLoading: true, error: null });

    try {
      const config = await invoke<AppConfig>("get_app_config");
      set({ ...applyConfig(config, get().selectedServerId), isLoading: false });
    } catch (error) {
      set({
        error: error instanceof Error ? error.message : String(error),
        isLoading: false,
      });
    }
  },

  selectServer: (serverId) => set({ selectedServerId: serverId }),

  upsertServer: async (server) => {
    const config = await invoke<AppConfig>("upsert_server", { server });
    set(applyConfig(config, server.id));
  },

  deleteServer: async (serverId) => {
    const config = await invoke<AppConfig>("delete_server", { serverId });
    set(applyConfig(
      config,
      get().selectedServerId === serverId ? config.servers[0]?.id ?? null : get().selectedServerId,
    ));
  },

  upsertBastion: async (bastion) => {
    const config = await invoke<AppConfig>("upsert_bastion", { bastion });
    set(applyConfig(config, get().selectedServerId));
  },

  deleteBastion: async (bastionId) => {
    const config = await invoke<AppConfig>("delete_bastion", { bastionId });
    set(applyConfig(config, get().selectedServerId));
  },

  savePollInterval: async (seconds) => {
    const config = await invoke<AppConfig>("set_poll_interval", { seconds });
    set(applyConfig(config, get().selectedServerId));
  },

  saveTheme: async (theme) => {
    const config = await invoke<AppConfig>("set_theme", { theme });
    set(applyConfig(config, get().selectedServerId));
  },

  saveMonitorTarget: async ({ serverId, bastionId }) => {
    const config = await invoke<AppConfig>("set_monitor_target", { serverId, bastionId });
    set(applyConfig(config, get().selectedServerId));
  },

  pingServer: async (serverId) => {
    try {
      const result = await invoke<PingResult>("ping_server", { serverId });
      set((state) => ({
        statusById: {
          ...state.statusById,
          [serverId]: softenTransientOffline(result, state.statusById[serverId]),
        },
      }));
    } catch (error) {
      const now = new Date().toISOString();
      set((state) => ({
        statusById: {
          ...state.statusById,
          [serverId]: softenTransientOffline(
            {
              serverId,
              latencyMs: null,
              status: "offline",
              message: error instanceof Error ? error.message : String(error),
              checkedAt: now,
            },
            state.statusById[serverId],
          ),
        },
      }));
    }
  },

  pingAllServers: async () => {
    if (get()._pingAllInFlight) return;
    set({ _pingAllInFlight: true });
    try {
      const { servers, pingServer } = get();
      await Promise.allSettled(servers.map((server) => pingServer(server.id)));
    } finally {
      set({ _pingAllInFlight: false });
    }
  },
}));

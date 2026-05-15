import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { AppConfig, AppTheme, BastionConfig, PingResult, ServerConfig } from "../types";

interface ServerState {
  servers: ServerConfig[];
  bastions: BastionConfig[];
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
  pingServer: (serverId: string) => Promise<void>;
  pingAllServers: () => Promise<void>;
}

const applyConfig = (
  config: AppConfig,
  currentSelected: string | null,
) => ({
  servers: config.servers,
  bastions: config.bastions ?? [],
  configPollIntervalSec: config.pollIntervalSec,
  theme: config.theme,
  selectedServerId:
    currentSelected && config.servers.some((server) => server.id === currentSelected)
      ? currentSelected
      : config.servers[0]?.id ?? null,
});

export const useServerStore = create<ServerState>((set, get) => ({
  servers: [],
  bastions: [],
  selectedServerId: null,
  statusById: {},
  configPollIntervalSec: 10,
  theme: "dark",
  isLoading: true,
  error: null,

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

  pingServer: async (serverId) => {
    try {
      const result = await invoke<PingResult>("ping_server", { serverId });
      set((state) => ({
        statusById: {
          ...state.statusById,
          [serverId]: result,
        },
      }));
    } catch (error) {
      const now = new Date().toISOString();
      set((state) => ({
        statusById: {
          ...state.statusById,
          [serverId]: {
            serverId,
            latencyMs: null,
            status: "offline",
            message: error instanceof Error ? error.message : String(error),
            checkedAt: now,
          },
        },
      }));
    }
  },

  pingAllServers: async () => {
    const { servers, pingServer } = get();
    await Promise.allSettled(servers.map((server) => pingServer(server.id)));
  },
}));

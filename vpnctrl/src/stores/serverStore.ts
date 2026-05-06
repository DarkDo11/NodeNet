import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { PingResult, ServerConfig } from "../types";

interface ServerState {
  servers: ServerConfig[];
  selectedServerId: string | null;
  statusById: Record<string, PingResult>;
  isLoading: boolean;
  error: string | null;
  loadServers: () => Promise<void>;
  selectServer: (serverId: string) => void;
  pingServer: (serverId: string) => Promise<void>;
  pingAllServers: () => Promise<void>;
}

export const useServerStore = create<ServerState>((set, get) => ({
  servers: [],
  selectedServerId: null,
  statusById: {},
  isLoading: false,
  error: null,

  loadServers: async () => {
    set({ isLoading: true, error: null });

    try {
      const servers = await invoke<ServerConfig[]>("get_servers");
      const currentSelected = get().selectedServerId;
      const selectedServerId =
        currentSelected && servers.some((server) => server.id === currentSelected)
          ? currentSelected
          : servers[0]?.id ?? null;

      set({ servers, selectedServerId, isLoading: false });
    } catch (error) {
      set({
        error: error instanceof Error ? error.message : String(error),
        isLoading: false,
      });
    }
  },

  selectServer: (serverId) => set({ selectedServerId: serverId }),

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

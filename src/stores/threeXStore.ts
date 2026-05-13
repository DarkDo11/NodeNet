import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type {
  GlobalTrafficStats,
  ServerConfig,
  ThreeXClient,
  ThreeXInbound,
  XrayConfig,
} from "../types";

interface ThreeXState {
  inboundsByServer: Record<string, ThreeXInbound[]>;
  clientsByInbound: Record<string, ThreeXClient[]>;
  xrayConfigByServer: Record<string, XrayConfig>;
  selectedInboundIdByServer: Record<string, number | null>;
  errorByServer: Record<string, string>;
  isLoadingInbounds: boolean;
  isLoadingClients: boolean;
  isLoadingXrayConfig: boolean;
  isSavingXrayConfig: boolean;
  isRunningAction: boolean;
  actionMessage: string;
  qrLink: string | null;
  qrTitle: string;
  globalStats: GlobalTrafficStats;
  selectInbound: (serverId: string, inboundId: number) => void;
  loadInbounds: (serverId: string) => Promise<void>;
  loadClients: (serverId: string, inboundId: number) => Promise<void>;
  loadXrayConfig: (serverId: string) => Promise<void>;
  saveXrayConfig: (serverId: string, config: XrayConfig) => Promise<void>;
  addClient: (
    serverId: string,
    inboundId: number,
    name: string,
    limitGb: number,
    expireDays: number,
  ) => Promise<void>;
  deleteClient: (serverId: string, inboundId: number, clientId: string) => Promise<void>;
  resetClientTraffic: (serverId: string, inboundId: number, clientId: string) => Promise<void>;
  resetAllExpired: (serverId: string, inboundId: number) => Promise<void>;
  deleteAllDisabled: (serverId: string, inboundId: number) => Promise<void>;
  exportClientsCsv: (serverId: string, inboundId: number) => Promise<void>;
  extendClient: (
    serverId: string,
    inboundId: number,
    clientId: string,
    days: number,
  ) => Promise<void>;
  generateClientLink: (
    serverId: string,
    inboundId: number,
    client: ThreeXClient,
  ) => Promise<void>;
  closeQr: () => void;
  restartXray: (serverId: string) => Promise<void>;
  rebootServer: (serverId: string) => Promise<void>;
  downloadConfig: (serverId: string) => Promise<void>;
  refreshGlobalStats: (servers: ServerConfig[]) => Promise<void>;
}

const emptyGlobalStats: GlobalTrafficStats = {
  dayUp: 0,
  dayDown: 0,
  monthUp: 0,
  monthDown: 0,
  updatedAt: null,
};

const inboundKey = (serverId: string, inboundId: number) => `${serverId}:${inboundId}`;

const totalTraffic = (inbounds: ThreeXInbound[]) =>
  inbounds.reduce(
    (acc, inbound) => ({
      up: acc.up + inbound.up,
      down: acc.down + inbound.down,
    }),
    { up: 0, down: 0 },
  );

const baselineValue = (key: string, current: number) => {
  const stored = Number(window.localStorage.getItem(key));
  if (!Number.isFinite(stored) || stored > current) {
    window.localStorage.setItem(key, String(current));
    return current;
  }
  return stored;
};

const statsFromTotals = (up: number, down: number): GlobalTrafficStats => {
  const now = new Date();
  const dayKey = now.toISOString().slice(0, 10);
  const monthKey = dayKey.slice(0, 7);
  const dayUpBase = baselineValue(`nodenet:traffic:${dayKey}:up`, up);
  const dayDownBase = baselineValue(`nodenet:traffic:${dayKey}:down`, down);
  const monthUpBase = baselineValue(`nodenet:traffic:${monthKey}:up`, up);
  const monthDownBase = baselineValue(`nodenet:traffic:${monthKey}:down`, down);

  return {
    dayUp: Math.max(0, up - dayUpBase),
    dayDown: Math.max(0, down - dayDownBase),
    monthUp: Math.max(0, up - monthUpBase),
    monthDown: Math.max(0, down - monthDownBase),
    updatedAt: now.toISOString(),
  };
};

export const useThreeXStore = create<ThreeXState>((set, get) => ({
  inboundsByServer: {},
  clientsByInbound: {},
  xrayConfigByServer: {},
  selectedInboundIdByServer: {},
  errorByServer: {},
  isLoadingInbounds: false,
  isLoadingClients: false,
  isLoadingXrayConfig: false,
  isSavingXrayConfig: false,
  isRunningAction: false,
  actionMessage: "",
  qrLink: null,
  qrTitle: "",
  globalStats: emptyGlobalStats,

  selectInbound: (serverId, inboundId) =>
    set((state) => ({
      selectedInboundIdByServer: {
        ...state.selectedInboundIdByServer,
        [serverId]: inboundId,
      },
    })),

  loadInbounds: async (serverId) => {
    set({ isLoadingInbounds: true });
    try {
      const inbounds = await invoke<ThreeXInbound[]>("get_inbounds", { serverId });
      const currentSelected = get().selectedInboundIdByServer[serverId];
      const selectedInboundId =
        currentSelected && inbounds.some((inbound) => inbound.id === currentSelected)
          ? currentSelected
          : inbounds[0]?.id ?? null;

      set((state) => ({
        inboundsByServer: {
          ...state.inboundsByServer,
          [serverId]: inbounds,
        },
        selectedInboundIdByServer: {
          ...state.selectedInboundIdByServer,
          [serverId]: selectedInboundId,
        },
        errorByServer: {
          ...state.errorByServer,
          [serverId]: "",
        },
        isLoadingInbounds: false,
      }));

      if (selectedInboundId !== null) {
        await get().loadClients(serverId, selectedInboundId);
      }
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        isLoadingInbounds: false,
      }));
    }
  },

  loadClients: async (serverId, inboundId) => {
    set({ isLoadingClients: true });
    try {
      const clients = await invoke<ThreeXClient[]>("get_clients", { serverId, inboundId });
      set((state) => ({
        clientsByInbound: {
          ...state.clientsByInbound,
          [inboundKey(serverId, inboundId)]: clients,
        },
        isLoadingClients: false,
      }));
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        isLoadingClients: false,
      }));
    }
  },

  loadXrayConfig: async (serverId) => {
    set({ isLoadingXrayConfig: true });
    try {
      const config = await invoke<XrayConfig>("get_xray_config", { serverId });
      set((state) => ({
        xrayConfigByServer: {
          ...state.xrayConfigByServer,
          [serverId]: config,
        },
        errorByServer: {
          ...state.errorByServer,
          [serverId]: "",
        },
        isLoadingXrayConfig: false,
      }));
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        isLoadingXrayConfig: false,
      }));
    }
  },

  saveXrayConfig: async (serverId, config) => {
    set({ isSavingXrayConfig: true, actionMessage: "" });
    try {
      await invoke("save_xray_config", { serverId, config });
      set((state) => ({
        xrayConfigByServer: {
          ...state.xrayConfigByServer,
          [serverId]: config,
        },
        errorByServer: {
          ...state.errorByServer,
          [serverId]: "",
        },
        isSavingXrayConfig: false,
        actionMessage: "Routing saved",
      }));
    } catch (error) {
      set((state) => ({
        errorByServer: {
          ...state.errorByServer,
          [serverId]: error instanceof Error ? error.message : String(error),
        },
        isSavingXrayConfig: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      }));
      throw error;
    }
  },

  addClient: async (serverId, inboundId, name, limitGb, expireDays) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke<ThreeXClient>("add_client", {
        serverId,
        inboundId,
        name,
        limitGb,
        expireDays,
      });
      await get().loadInbounds(serverId);
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: "Client added" });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  deleteClient: async (serverId, inboundId, clientId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke("delete_client", { serverId, inboundId, clientId });
      await get().loadInbounds(serverId);
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: "Client deleted" });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  resetClientTraffic: async (serverId, inboundId, clientId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke("reset_client_traffic", { serverId, inboundId, clientId });
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: "Traffic reset" });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  resetAllExpired: async (serverId, inboundId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      const count = await invoke<number>("reset_all_expired_clients", { serverId, inboundId });
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: `Reset ${count} expired clients` });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  deleteAllDisabled: async (serverId, inboundId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      const count = await invoke<number>("delete_all_disabled_clients", { serverId, inboundId });
      await get().loadInbounds(serverId);
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: `Deleted ${count} disabled clients` });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  exportClientsCsv: async (serverId, inboundId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      const path = await invoke<string>("export_clients_csv", { serverId, inboundId });
      set({ isRunningAction: false, actionMessage: `CSV exported: ${path}` });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  extendClient: async (serverId, inboundId, clientId, days) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke<ThreeXClient>("extend_client", {
        serverId,
        inboundId,
        clientId,
        days,
      });
      await get().loadClients(serverId, inboundId);
      set({ isRunningAction: false, actionMessage: `Extended by ${days} days` });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  generateClientLink: async (serverId, inboundId, client) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      const link = await invoke<string>("generate_client_link", {
        serverId,
        inboundId,
        clientId: client.id,
      });
      set({
        qrLink: link,
        qrTitle: client.email,
        isRunningAction: false,
      });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  closeQr: () => set({ qrLink: null, qrTitle: "" }),

  restartXray: async (serverId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke("restart_xray", { serverId });
      set({ isRunningAction: false, actionMessage: "Xray restarted" });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  rebootServer: async (serverId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      await invoke("reboot_server", { serverId });
      set({ isRunningAction: false, actionMessage: "Reboot command sent" });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  downloadConfig: async (serverId) => {
    set({ isRunningAction: true, actionMessage: "" });
    try {
      const path = await invoke<string>("download_config", { serverId });
      set({ isRunningAction: false, actionMessage: `Backup saved: ${path}` });
    } catch (error) {
      set({
        isRunningAction: false,
        actionMessage: error instanceof Error ? error.message : String(error),
      });
    }
  },

  refreshGlobalStats: async (servers) => {
    const panelServers = servers.filter((server) => server.panelUrl);
    if (panelServers.length === 0) {
      set({ globalStats: emptyGlobalStats });
      return;
    }

    const results = await Promise.allSettled(
      panelServers.map((server) =>
        invoke<ThreeXInbound[]>("get_inbounds", { serverId: server.id }),
      ),
    );
    if (results.some((result) => result.status === "rejected")) {
      return;
    }

    const totals = results
      .filter((result): result is PromiseFulfilledResult<ThreeXInbound[]> => result.status === "fulfilled")
      .map((result) => totalTraffic(result.value))
      .reduce(
        (acc, item) => ({
          up: acc.up + item.up,
          down: acc.down + item.down,
        }),
        { up: 0, down: 0 },
      );

    set({ globalStats: statsFromTotals(totals.up, totals.down) });
  },
}));

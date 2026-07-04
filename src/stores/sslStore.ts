import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { ServerCertificates, ServerConfig, SslCertificate } from "../types";

export interface SslCertificateRow extends SslCertificate {
  serverId: string;
  serverName: string;
}

interface SslState {
  certificates: SslCertificateRow[];
  certbotInstalledByServer: Record<string, boolean>;
  errorByServer: Record<string, string>;
  isLoading: boolean;
  _loadAllInFlight: boolean;
  loadAllCertificates: (servers: ServerConfig[]) => Promise<void>;
}

const expiryTime = (row: SslCertificateRow) =>
  row.expiresAt ? new Date(row.expiresAt).getTime() : Number.POSITIVE_INFINITY;

export const useSslStore = create<SslState>((set, get) => ({
  certificates: [],
  certbotInstalledByServer: {},
  errorByServer: {},
  isLoading: false,
  _loadAllInFlight: false,

  loadAllCertificates: async (servers) => {
    if (get()._loadAllInFlight) return;
    set({ _loadAllInFlight: true, isLoading: true });

    const results = await Promise.allSettled(
      servers.map((server) =>
        invoke<ServerCertificates>("list_ssl_certificates", { serverId: server.id }),
      ),
    );

    const certificates: SslCertificateRow[] = [];
    const certbotInstalledByServer: Record<string, boolean> = {};
    const errorByServer: Record<string, string> = {};

    results.forEach((result, index) => {
      const server = servers[index];
      if (result.status === "fulfilled") {
        certbotInstalledByServer[server.id] = result.value.certbotInstalled;
        for (const certificate of result.value.certificates) {
          certificates.push({ ...certificate, serverId: server.id, serverName: server.name });
        }
      } else {
        errorByServer[server.id] =
          result.reason instanceof Error ? result.reason.message : String(result.reason);
      }
    });

    certificates.sort((a, b) => expiryTime(a) - expiryTime(b));

    set({
      certificates,
      certbotInstalledByServer,
      errorByServer,
      isLoading: false,
      _loadAllInFlight: false,
    });
  },
}));

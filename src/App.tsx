import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Clients from "./components/Clients";
import ConfirmModal from "./components/ConfirmModal";
import Dashboard from "./components/Dashboard";
import EventsLog from "./components/EventsLog";
import Inbounds from "./components/Inbounds";
import Onboarding from "./components/Onboarding";
import QrModal from "./components/QrModal";
import Settings from "./components/Settings";
import Sidebar, { type AppView } from "./components/Sidebar";
import TerminalView from "./components/TerminalView";
import ToastHost from "./components/ToastHost";
import Topbar from "./components/Topbar";
import { useEventsStore } from "./stores/eventsStore";
import { useMetricsStore } from "./stores/metricsStore";
import { useServerStore } from "./stores/serverStore";
import { useThreeXStore } from "./stores/threeXStore";
import type { AppTheme, MetricPoint, ServerConfig, ThreeXClient } from "./types";

export default function App() {
  const [activeView, setActiveView] = useState<AppView>("dashboard");
  const {
    servers,
    selectedServerId,
    statusById,
    configPollIntervalSec,
    theme,
    isLoading: isLoadingServers,
    loadServers,
    selectServer,
    upsertServer,
    deleteServer,
    savePollInterval,
    saveTheme,
    pingAllServers,
  } = useServerStore();
  const {
    metricsByServer,
    historyByServer,
    pollIntervalSec,
    isPollingServer,
    errorByServer,
    setPollInterval,
    fetchMetrics,
    loadMetricsCache,
  } = useMetricsStore();
  const {
    inboundsByServer,
    clientsByInbound,
    selectedInboundIdByServer,
    errorByServer: panelErrorByServer,
    isLoadingInbounds,
    isLoadingClients,
    isRunningAction,
    actionMessage,
    qrLink,
    qrTitle,
    globalStats,
    selectInbound,
    loadInbounds,
    loadClients,
    addClient,
    deleteClient,
    resetClientTraffic,
    resetAllExpired,
    deleteAllDisabled,
    exportClientsCsv,
    extendClient,
    generateClientLink,
    closeQr,
    restartXray,
    rebootServer,
    downloadConfig,
    refreshGlobalStats,
  } = useThreeXStore();
  const {
    events,
    toasts,
    error: eventsError,
    loadEvents,
    attachAlertListeners,
    dismissToast,
  } = useEventsStore();
  const [confirm, setConfirm] = useState<{
    title: string;
    message: string;
    confirmLabel: string;
    onConfirm: () => void;
  } | null>(null);

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [selectedServerId, servers],
  );

  const latestMetricsByServer = useMemo(() => {
    return Object.fromEntries(
      Object.entries(historyByServer).map(([serverId, history]) => [
        serverId,
        history[history.length - 1],
      ]),
    ) as Record<string, MetricPoint | undefined>;
  }, [historyByServer]);

  const selectedInboundId = selectedServerId
    ? selectedInboundIdByServer[selectedServerId] ?? null
    : null;
  const selectedInbounds = selectedServerId ? inboundsByServer[selectedServerId] ?? [] : [];
  const selectedInbound =
    selectedInboundId !== null
      ? selectedInbounds.find((inbound) => inbound.id === selectedInboundId) ?? null
      : null;
  const selectedClients =
    selectedServerId && selectedInboundId !== null
      ? clientsByInbound[`${selectedServerId}:${selectedInboundId}`] ?? []
      : [];

  useEffect(() => {
    void loadServers();
    void loadMetricsCache();
  }, [loadMetricsCache, loadServers]);

  useEffect(() => {
    setPollInterval(configPollIntervalSec);
  }, [configPollIntervalSec, setPollInterval]);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const resolvedTheme = theme === "system" ? (media.matches ? "dark" : "light") : theme;
      document.documentElement.dataset.theme = resolvedTheme;
    };

    applyTheme();
    if (theme !== "system") return;

    media.addEventListener("change", applyTheme);
    return () => media.removeEventListener("change", applyTheme);
  }, [theme]);

  useEffect(() => {
    const title = selectedServer ? `${selectedServer.name} — NodeNet` : "NodeNet";
    void getCurrentWindow().setTitle(title);
  }, [selectedServer]);

  useEffect(() => {
    void loadEvents();
    let cleanup: (() => void) | null = null;
    void attachAlertListeners().then((unlisten) => {
      cleanup = unlisten;
    });

    return () => {
      cleanup?.();
    };
  }, [attachAlertListeners, loadEvents]);

  useEffect(() => {
    if (servers.length === 0) return;

    void pingAllServers();
    const timer = window.setInterval(() => {
      void pingAllServers();
    }, Math.max(2, pollIntervalSec) * 1000);

    return () => window.clearInterval(timer);
  }, [pingAllServers, pollIntervalSec, servers.length]);

  useEffect(() => {
    if (!selectedServerId) return;

    void fetchMetrics(selectedServerId);
    const timer = window.setInterval(() => {
      void fetchMetrics(selectedServerId);
    }, Math.max(2, pollIntervalSec) * 1000);

    return () => window.clearInterval(timer);
  }, [fetchMetrics, pollIntervalSec, selectedServerId]);

  useEffect(() => {
    if (!selectedServerId || !selectedServer?.panelUrl) return;

    void loadInbounds(selectedServerId);
  }, [loadInbounds, selectedServer?.panelUrl, selectedServerId]);

  useEffect(() => {
    if (servers.length === 0) return;

    void refreshGlobalStats(servers);
    const timer = window.setInterval(() => {
      void refreshGlobalStats(servers);
    }, Math.max(10, pollIntervalSec) * 1000);

    return () => window.clearInterval(timer);
  }, [pollIntervalSec, refreshGlobalStats, servers]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.metaKey || event.altKey || event.ctrlKey || event.shiftKey) return;
      if (!/^[1-9]$/.test(event.key)) return;

      const index = Number(event.key) - 1;
      const server = servers[index];
      if (!server) return;

      event.preventDefault();
      selectServer(server.id);
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectServer, servers]);

  const saveServer = async (server: ServerConfig) => {
    await upsertServer(server);
    selectServer(server.id);
  };

  const updatePollInterval = async (seconds: number) => {
    setPollInterval(seconds);
    await savePollInterval(seconds);
  };

  const updateTheme = async (nextTheme: AppTheme) => {
    await saveTheme(nextTheme);
  };

  const showOnboarding = !isLoadingServers && servers.length === 0 && activeView !== "settings";

  if (isLoadingServers && servers.length === 0) {
    return (
      <main className="app-loading">
        <section className="app-loading-panel">
          <span className="skeleton-line short" />
          <span className="skeleton-line tall" />
          <span className="skeleton-line" />
          <span className="skeleton-line" />
        </section>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <Sidebar
        servers={servers}
        selectedServerId={selectedServerId}
        statusById={statusById}
        latestMetricsByServer={latestMetricsByServer}
        activeView={activeView}
        onSelectServer={(serverId) => {
          selectServer(serverId);
          setActiveView("dashboard");
        }}
        onChangeView={setActiveView}
      />
      <div className="main-area">
        <Topbar
          server={selectedServer}
          stats={globalStats}
          isRunningAction={isRunningAction}
          message={actionMessage}
          onRestartXray={() => {
            if (selectedServerId) void restartXray(selectedServerId);
          }}
          onReboot={() => {
            if (selectedServerId && selectedServer) {
              setConfirm({
                title: `Reboot ${selectedServer.name}?`,
                message: `Reboot ${selectedServer.name}? All connections will drop.`,
                confirmLabel: "Reboot",
                onConfirm: () => void rebootServer(selectedServerId),
              });
            }
          }}
          onBackup={() => {
            if (selectedServerId) void downloadConfig(selectedServerId);
          }}
        />
        {showOnboarding ? (
          <Onboarding onCreateServer={saveServer} />
        ) : null}
        {!showOnboarding && activeView === "dashboard" ? (
          <Dashboard
            server={selectedServer}
            metrics={selectedServerId ? metricsByServer[selectedServerId] : undefined}
            history={selectedServerId ? historyByServer[selectedServerId] ?? [] : []}
            status={selectedServerId ? statusById[selectedServerId] : undefined}
            error={selectedServerId ? errorByServer[selectedServerId] : undefined}
            isPolling={selectedServerId ? isPollingServer(selectedServerId) : false}
            onRetry={() => {
              if (selectedServerId) void fetchMetrics(selectedServerId);
            }}
          />
        ) : null}
        {!showOnboarding && activeView === "inbounds" ? (
          <Inbounds
            server={selectedServer}
            inbounds={selectedInbounds}
            selectedInboundId={selectedInboundId}
            error={selectedServerId ? panelErrorByServer[selectedServerId] : undefined}
            isLoading={isLoadingInbounds}
            isRunningAction={isRunningAction}
            onRefresh={() => {
              if (selectedServerId) void loadInbounds(selectedServerId);
            }}
            onSelectInbound={(inboundId) => {
              if (!selectedServerId) return;
              selectInbound(selectedServerId, inboundId);
              void loadClients(selectedServerId, inboundId);
              setActiveView("clients");
            }}
            onRestartXray={() => {
              if (selectedServerId) void restartXray(selectedServerId);
            }}
          />
        ) : null}
        {!showOnboarding && activeView === "clients" ? (
          <Clients
            server={selectedServer}
            inbound={selectedInbound}
            clients={selectedClients}
            error={selectedServerId ? panelErrorByServer[selectedServerId] : undefined}
            isLoading={isLoadingClients}
            isRunningAction={isRunningAction}
            onRefresh={() => {
              if (selectedServerId && selectedInboundId !== null) {
                void loadClients(selectedServerId, selectedInboundId);
              }
            }}
            onAddClient={(name, limitGb, expireDays) => {
              if (selectedServerId && selectedInboundId !== null) {
                void addClient(selectedServerId, selectedInboundId, name, limitGb, expireDays);
              }
            }}
            onReset={(client: ThreeXClient) => {
              if (selectedServerId && selectedInboundId !== null) {
                void resetClientTraffic(selectedServerId, selectedInboundId, client.id);
              }
            }}
            onDelete={(client: ThreeXClient) => {
              if (selectedServerId && selectedInboundId !== null) {
                setConfirm({
                  title: `Delete ${client.email}?`,
                  message: `Delete ${client.email}? This cannot be undone.`,
                  confirmLabel: "Delete client",
                  onConfirm: () => void deleteClient(selectedServerId, selectedInboundId, client.id),
                });
              }
            }}
            onExtend={(client: ThreeXClient, days) => {
              if (selectedServerId && selectedInboundId !== null) {
                void extendClient(selectedServerId, selectedInboundId, client.id, days);
              }
            }}
            onQr={(client: ThreeXClient) => {
              if (selectedServerId && selectedInboundId !== null) {
                void generateClientLink(selectedServerId, selectedInboundId, client);
              }
            }}
            onResetAllExpired={() => {
              if (selectedServerId && selectedInboundId !== null) {
                void resetAllExpired(selectedServerId, selectedInboundId);
              }
            }}
            onDeleteAllDisabled={() => {
              if (selectedServerId && selectedInboundId !== null) {
                setConfirm({
                  title: "Delete all disabled clients?",
                  message: "Delete all disabled clients? This cannot be undone.",
                  confirmLabel: "Delete disabled",
                  onConfirm: () => void deleteAllDisabled(selectedServerId, selectedInboundId),
                });
              }
            }}
            onExportCsv={() => {
              if (selectedServerId && selectedInboundId !== null) {
                void exportClientsCsv(selectedServerId, selectedInboundId);
              }
            }}
          />
        ) : null}
        {!showOnboarding ? (
          <div className="view-slot" style={{ display: activeView === "terminal" ? "flex" : "none" }}>
            <TerminalView server={selectedServer} />
          </div>
        ) : null}
        {!showOnboarding && activeView === "events" ? (
          <EventsLog
            events={events}
            error={eventsError}
            onRefresh={() => {
              void loadEvents();
            }}
          />
        ) : null}
        {!showOnboarding && activeView === "settings" ? (
          <Settings
            servers={servers}
            pollIntervalSec={pollIntervalSec}
            theme={theme}
            onPollIntervalChange={updatePollInterval}
            onThemeChange={updateTheme}
            onSaveServer={saveServer}
            onDeleteServer={deleteServer}
          />
        ) : null}
      </div>
      <QrModal title={qrTitle} link={qrLink} onClose={closeQr} />
      {confirm ? (
        <ConfirmModal
          title={confirm.title}
          message={confirm.message}
          confirmLabel={confirm.confirmLabel}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            const action = confirm.onConfirm;
            setConfirm(null);
            action();
          }}
        />
      ) : null}
      <ToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}

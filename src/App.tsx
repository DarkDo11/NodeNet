import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Clients from "./components/Clients";
import ConfirmModal from "./components/ConfirmModal";
import Dashboard from "./components/Dashboard";
import EventsLog from "./components/EventsLog";
import Inbounds from "./components/Inbounds";
import LogsView from "./components/LogsView";
import Onboarding from "./components/Onboarding";
import QrModal from "./components/QrModal";
import RoutingEditor from "./components/RoutingEditor";
import Settings from "./components/Settings";
import Sidebar, { type AppView } from "./components/Sidebar";
import SslCertificates from "./components/SslCertificates";
import TerminalView from "./components/TerminalView";
import ToastHost from "./components/ToastHost";
import Topbar from "./components/Topbar";
import UpdateChecker from "./components/UpdateChecker";
import { useEventsStore } from "./stores/eventsStore";
import { useMetricsStore } from "./stores/metricsStore";
import { useServerStore } from "./stores/serverStore";
import { useThreeXStore } from "./stores/threeXStore";
import type { AppTheme, MetricPoint, ServerConfig, ThreeXClient } from "./types";

const WINDOW_DRAG_SELECTOR = "[data-window-drag], [data-tauri-drag-region]";
const NO_WINDOW_DRAG_SELECTOR =
  "button, input, select, textarea, a, [contenteditable='true'], [data-no-window-drag]";

const shouldStartWindowDrag = (event: PointerEvent) => {
  if (event.button !== 0) return false;
  const target = event.target as HTMLElement | null;
  if (!target?.closest(WINDOW_DRAG_SELECTOR)) return false;
  return !target.closest(NO_WINDOW_DRAG_SELECTOR);
};

export default function App() {
  const [activeView, setActiveView] = useState<AppView>("dashboard");
  const [onboardingSetupServerId, setOnboardingSetupServerId] = useState<string | null>(null);
  const {
    servers,
    bastions,
    monitorServerId,
    monitorBastionId,
    selectedServerId,
    statusById,
    configPollIntervalSec,
    theme,
    isLoading: isLoadingServers,
    loadServers,
    selectServer,
    upsertServer,
    deleteServer,
    upsertBastion,
    deleteBastion,
    savePollInterval,
    saveTheme,
    saveMonitorTarget,
    pingAllServers,
  } = useServerStore();
  const {
    metricsByServer,
    historyByServer,
    selectedRange,
    pollIntervalSec,
    isPollingServer,
    errorByServer,
    setPollInterval,
    setSelectedRange,
    fetchMetrics,
    loadMetricsCache,
    getMetricsForRange,
    getUptimeForRange,
  } = useMetricsStore();
  const {
    inboundsByServer,
    clientsByInbound,
    xrayConfigByServer,
    selectedInboundIdByServer,
    errorByServer: panelErrorByServer,
    loadingInboundsById,
    loadingClientsById,
    isLoadingXrayConfig,
    isSavingXrayConfig,
    runningActionById,
    actionMessage,
    qrLink,
    qrTitle,
    globalStats,
    selectInbound,
    loadInbounds,
    loadClients,
    loadXrayConfig,
    saveXrayConfig,
    uploadRoutingFile,
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
    pushToast,
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

  const isLoadingInbounds = selectedServerId ? (loadingInboundsById[selectedServerId] ?? false) : false;
  const isLoadingClients = selectedServerId ? (loadingClientsById[selectedServerId] ?? false) : false;
  const isRunningAction = selectedServerId ? (runningActionById[selectedServerId] ?? false) : false;

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
  const selectedMetricsHistory = useMemo(
    () => (selectedServerId ? getMetricsForRange(selectedServerId, selectedRange) : []),
    [getMetricsForRange, selectedRange, selectedServerId, historyByServer],
  );
  const selectedUptimeSummary = useMemo(
    () =>
      selectedServerId
        ? getUptimeForRange(selectedServerId, selectedRange)
        : { percent: null, offlineEvents: 0, totalPoints: 0 },
    [getUptimeForRange, selectedRange, selectedServerId, historyByServer],
  );

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
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    void loadEvents();
    void attachAlertListeners().then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    const timer = window.setInterval(() => void loadEvents(), Math.max(10, pollIntervalSec) * 1000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [attachAlertListeners, loadEvents, pollIntervalSec]);

  useEffect(() => {
    if (monitorServerId || monitorBastionId) void loadMetricsCache();
  }, [monitorServerId, monitorBastionId, loadMetricsCache]);

  useEffect(() => {
    if (!monitorServerId && !monitorBastionId) return;
    const timer = window.setInterval(
      () => void loadMetricsCache(),
      Math.max(10, pollIntervalSec) * 1000,
    );
    return () => window.clearInterval(timer);
  }, [loadMetricsCache, monitorBastionId, monitorServerId, pollIntervalSec]);

  useEffect(() => {
    if (servers.length > 0) void pingAllServers();
  }, [servers.length, pingAllServers]);

  useEffect(() => {
    if (servers.length === 0) return;
    const timer = window.setInterval(
      () => void pingAllServers(),
      Math.max(2, pollIntervalSec) * 1000,
    );
    return () => window.clearInterval(timer);
  }, [pingAllServers, pollIntervalSec, servers.length]);

  useEffect(() => {
    if (selectedServerId) void fetchMetrics(selectedServerId);
  }, [selectedServerId, fetchMetrics]);

  useEffect(() => {
    if (!selectedServerId) return;
    const timer = window.setInterval(
      () => void fetchMetrics(selectedServerId),
      Math.max(2, pollIntervalSec) * 1000,
    );
    return () => window.clearInterval(timer);
  }, [fetchMetrics, pollIntervalSec, selectedServerId]);

  useEffect(() => {
    if (!selectedServerId || !selectedServer?.panelUrl) return;

    void loadInbounds(selectedServerId);
  }, [loadInbounds, selectedServer?.panelUrl, selectedServerId]);

  useEffect(() => {
    if (activeView !== "routing" || !selectedServerId || !selectedServer?.panelUrl) return;

    void loadXrayConfig(selectedServerId);
  }, [activeView, loadXrayConfig, selectedServer?.panelUrl, selectedServerId]);

  const serverIds = servers.map((s) => s.id).join(",");
  useEffect(() => {
    if (servers.length > 0) void refreshGlobalStats(servers);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverIds, refreshGlobalStats]);

  useEffect(() => {
    if (servers.length === 0) return;
    const timer = window.setInterval(
      () => void refreshGlobalStats(servers),
      Math.max(30, pollIntervalSec) * 1000,
    );
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pollIntervalSec, refreshGlobalStats, serverIds]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.metaKey || event.altKey || event.ctrlKey || event.shiftKey) return;
      if (!/^[1-9]$/.test(event.key)) return;
      if (confirm !== null) return;

      const index = Number(event.key) - 1;
      const server = servers[index];
      if (!server) return;

      event.preventDefault();
      selectServer(server.id);
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [confirm, selectServer, servers]);

  useEffect(() => {
    const unlistenPromise = listen<string>("tray-select-server", (event) => {
      selectServer(event.payload);
    });
    return () => { void unlistenPromise.then((unlisten) => unlisten()); };
  }, [selectServer]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!shouldStartWindowDrag(event)) return;
      void getCurrentWindow().startDragging();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, []);

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

  const showOnboarding =
    (!isLoadingServers && servers.length === 0 && activeView !== "settings") ||
    onboardingSetupServerId !== null;

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
      <div className="window-drag-strip" data-window-drag data-tauri-drag-region />
      <Sidebar
        servers={servers}
        selectedServerId={selectedServerId}
        statusById={statusById}
        latestMetricsByServer={latestMetricsByServer}
        activeView={activeView}
        onSelectServer={(serverId) => {
          selectServer(serverId);
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
        <div className="view-slot">
          {showOnboarding ? (
            <Onboarding
              bastions={bastions}
              onCreateServer={saveServer}
              onSaveBastion={upsertBastion}
              onSetupStarted={setOnboardingSetupServerId}
              onFinishSetup={() => {
                setOnboardingSetupServerId(null);
                setActiveView("dashboard");
              }}
            />
          ) : null}
          {!showOnboarding && activeView === "dashboard" ? (
            <Dashboard
              server={selectedServer}
              metrics={selectedServerId ? metricsByServer[selectedServerId] : undefined}
              history={selectedMetricsHistory}
              selectedRange={selectedRange}
              uptimeSummary={selectedUptimeSummary}
              status={selectedServerId ? statusById[selectedServerId] : undefined}
              error={selectedServerId ? errorByServer[selectedServerId] : undefined}
              isPolling={selectedServerId ? isPollingServer(selectedServerId) : false}
              onRangeChange={setSelectedRange}
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
          {!showOnboarding && activeView === "routing" ? (
            <RoutingEditor
              server={selectedServer}
              config={selectedServerId ? xrayConfigByServer[selectedServerId] ?? null : null}
              error={selectedServerId ? panelErrorByServer[selectedServerId] : undefined}
              isLoading={isLoadingXrayConfig}
              isSaving={isSavingXrayConfig}
              isUploading={isRunningAction}
              onRefresh={() => {
                if (selectedServerId) void loadXrayConfig(selectedServerId);
              }}
              onSave={async (config) => {
                if (!selectedServerId) return;
                try {
                  await saveXrayConfig(selectedServerId, config);
                  pushToast("info", "Xray routing saved and restarted");
                } catch (error) {
                  pushToast("error", error instanceof Error ? error.message : String(error));
                  throw error;
                }
              }}
              onUploadRoutingFile={async (localPath, remoteFilename) => {
                if (!selectedServerId) return "";
                try {
                  const remotePath = await uploadRoutingFile(selectedServerId, localPath, remoteFilename);
                  pushToast("info", "Routing file uploaded and Xray restarted");
                  return remotePath;
                } catch (error) {
                  pushToast("error", error instanceof Error ? error.message : String(error));
                  throw error;
                }
              }}
            />
          ) : null}
          {!showOnboarding && activeView === "terminal" ? <TerminalView server={selectedServer} /> : null}
          {!showOnboarding && activeView === "events" ? (
            <EventsLog
              events={events}
              error={eventsError}
              onRefresh={() => {
                void loadEvents();
              }}
            />
          ) : null}
          {!showOnboarding && activeView === "logs" ? (
            <LogsView
              servers={servers}
              bastions={bastions}
              monitorServerId={monitorServerId}
              monitorBastionId={monitorBastionId}
              selectedServerId={selectedServerId}
            />
          ) : null}
          {!showOnboarding && activeView === "ssl" ? <SslCertificates servers={servers} /> : null}
          {!showOnboarding && activeView === "settings" ? (
            <Settings
              servers={servers}
              bastions={bastions}
              monitorServerId={monitorServerId}
              monitorBastionId={monitorBastionId}
              pollIntervalSec={pollIntervalSec}
              theme={theme}
              onPollIntervalChange={updatePollInterval}
              onThemeChange={updateTheme}
              onMonitorTargetChange={saveMonitorTarget}
              onSaveServer={saveServer}
              onDeleteServer={deleteServer}
              onSaveBastion={upsertBastion}
              onDeleteBastion={deleteBastion}
            />
          ) : null}
        </div>
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
      <UpdateChecker />
      <ToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}

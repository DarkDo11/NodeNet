import { useEffect, useMemo, useState } from "react";
import Clients from "./components/Clients";
import Dashboard from "./components/Dashboard";
import EventsLog from "./components/EventsLog";
import Inbounds from "./components/Inbounds";
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
import type { MetricPoint, ThreeXClient } from "./types";

export default function App() {
  const [activeView, setActiveView] = useState<AppView>("dashboard");
  const {
    servers,
    selectedServerId,
    statusById,
    loadServers,
    selectServer,
    pingAllServers,
  } = useServerStore();
  const {
    metricsByServer,
    historyByServer,
    pollIntervalSec,
    isPolling,
    errorByServer,
    setPollInterval,
    fetchMetrics,
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

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [selectedServerId, servers],
  );

  const latestMetricsByServer = useMemo(() => {
    return Object.fromEntries(
      Object.entries(historyByServer).map(([serverId, history]) => [
        serverId,
        history.at(-1),
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
  }, [loadServers]);

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
            if (selectedServerId) void rebootServer(selectedServerId);
          }}
          onBackup={() => {
            if (selectedServerId) void downloadConfig(selectedServerId);
          }}
        />
        {activeView === "dashboard" ? (
          <Dashboard
            server={selectedServer}
            metrics={selectedServerId ? metricsByServer[selectedServerId] : undefined}
            history={selectedServerId ? historyByServer[selectedServerId] ?? [] : []}
            status={selectedServerId ? statusById[selectedServerId] : undefined}
            error={selectedServerId ? errorByServer[selectedServerId] : undefined}
            isPolling={isPolling}
          />
        ) : null}
        {activeView === "inbounds" ? (
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
        {activeView === "clients" ? (
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
                void deleteClient(selectedServerId, selectedInboundId, client.id);
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
          />
        ) : null}
        {activeView === "terminal" ? <TerminalView server={selectedServer} /> : null}
        {activeView === "events" ? (
          <EventsLog
            events={events}
            error={eventsError}
            onRefresh={() => {
              void loadEvents();
            }}
          />
        ) : null}
        {activeView === "settings" ? (
          <Settings
            servers={servers}
            pollIntervalSec={pollIntervalSec}
            onPollIntervalChange={setPollInterval}
          />
        ) : null}
      </div>
      <QrModal title={qrTitle} link={qrLink} onClose={closeQr} />
      <ToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}

import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import {
  Download,
  KeyRound,
  Plus,
  RefreshCw,
  Save,
  ServerCog,
  ShieldCheck,
  SlidersHorizontal,
  Wifi,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import ConfirmModal from "./ConfirmModal";
import CountryFlag from "./CountryFlag";
import SetupPresets from "./SetupPresets";
import type { AppTheme, BastionConfig, PanelSetupInfo, ServerConfig, TestConnectionResult } from "../types";

interface SettingsProps {
  servers: ServerConfig[];
  bastions: BastionConfig[];
  monitorServerId: string | null;
  monitorBastionId: string | null;
  pollIntervalSec: number;
  theme: AppTheme;
  onPollIntervalChange: (seconds: number) => Promise<void>;
  onThemeChange: (theme: AppTheme) => Promise<void>;
  onMonitorTargetChange: (target: { serverId: string | null; bastionId: string | null }) => Promise<void>;
  onSaveServer: (server: ServerConfig) => Promise<void>;
  onDeleteServer: (serverId: string) => Promise<void>;
  onSaveBastion: (bastion: BastionConfig) => Promise<void>;
  onDeleteBastion: (bastionId: string) => Promise<void>;
}

const emptyServer = (): ServerConfig => ({
  id: "",
  name: "",
  host: "",
  sshPort: 22,
  sshUser: "root",
  country: "US",
  panelUrl: "",
  panelUser: "admin",
  sshKeyPath: "",
  bastionHost: "",
  bastionPort: 22,
  bastionUser: "",
  bastionSshKeyPath: "",
  sshKeyPassphrase: null,
  sslVerify: false,
});

const slug = (value: string) =>
  value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 36);

const bastionFromServer = (server: ServerConfig): BastionConfig | null => {
  const host = server.bastionHost?.trim();
  if (!host) return null;

  return {
    id: "",
    name: host,
    host,
    port: server.bastionPort || 22,
    user: server.bastionUser?.trim() || server.sshUser.trim() || "root",
    sshKeyPath: server.bastionSshKeyPath?.trim() || null,
  };
};

const normalizePanelBasePath = (value: string | null | undefined) => {
  const trimmed = (value ?? "").trim().replace(/^\/+|\/+$/g, "");
  return trimmed ? `/${trimmed}` : "";
};

export default function Settings({
  servers,
  bastions,
  monitorServerId,
  monitorBastionId,
  pollIntervalSec,
  theme,
  onPollIntervalChange,
  onThemeChange,
  onMonitorTargetChange,
  onSaveServer,
  onDeleteServer,
  onSaveBastion,
  onDeleteBastion,
}: SettingsProps) {
  const [selectedServerId, setSelectedServerId] = useState<string>("");
  const [isCreatingServer, setIsCreatingServer] = useState(false);
  const [form, setForm] = useState<ServerConfig>(emptyServer);
  const [password, setPassword] = useState("");
  const [keyPassphrase, setKeyPassphrase] = useState("");
  const [bastionPassword, setBastionPassword] = useState("");
  const [selectedBastionId, setSelectedBastionId] = useState("");
  const [bastionPresetName, setBastionPresetName] = useState("");
  const [syncMonitorKey, setSyncMonitorKey] = useState(true);
  const [panelPassword, setPanelPassword] = useState("");
  const [setupServerId, setSetupServerId] = useState<string | null>(null);
  const [configPath, setConfigPath] = useState("");
  const [appVersion, setAppVersion] = useState("");
  const [updateStatus, setUpdateStatus] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [testing, setTesting] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingMonitor, setInstallingMonitor] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [selectedServerId, servers],
  );
  const setupServer = useMemo(
    () => servers.find((server) => server.id === setupServerId) ?? null,
    [setupServerId, servers],
  );
  const monitorTargetValue = monitorServerId
    ? `server:${monitorServerId}`
    : monitorBastionId
      ? `bastion:${monitorBastionId}`
      : "";
  const hasMonitor = Boolean(monitorServerId || monitorBastionId);

  useEffect(() => {
    if (!selectedServerId && servers[0] && !isCreatingServer) {
      setSelectedServerId(servers[0].id);
    }
  }, [isCreatingServer, selectedServerId, servers]);

  useEffect(() => {
    setForm(selectedServer ? { ...emptyServer(), ...selectedServer } : emptyServer());
    const selectedBastion = selectedServer ? bastionFromServer(selectedServer) : null;
    const matchingBastion = selectedBastion
      ? bastions.find(
          (bastion) =>
            bastion.host === selectedBastion.host &&
            bastion.port === selectedBastion.port &&
            bastion.user === selectedBastion.user &&
            (bastion.sshKeyPath ?? "") === (selectedBastion.sshKeyPath ?? ""),
        )
      : null;
    setSelectedBastionId(matchingBastion?.id ?? "");
    setBastionPresetName(matchingBastion?.name ?? selectedBastion?.name ?? "");
    setSyncMonitorKey(!selectedServer);
    setPassword("");
    setKeyPassphrase("");
    setBastionPassword("");
    setPanelPassword("");
  }, [bastions, selectedServer]);

  useEffect(() => {
    void invoke<string>("get_config_path")
      .then(setConfigPath)
      .catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }, []);

  useEffect(() => {
    void getVersion()
      .then(setAppVersion)
      .catch((err) => setUpdateStatus(err instanceof Error ? err.message : String(err)));
  }, []);

  const updateForm = <K extends keyof ServerConfig>(key: K, value: ServerConfig[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const normalizedServer = () => {
    const bastionHost = form.bastionHost?.trim() || null;
    return {
      ...form,
      id: form.id.trim() || slug(`${form.name}-${form.host}`) || crypto.randomUUID(),
      name: form.name.trim(),
      host: form.host.trim(),
      sshUser: form.sshUser.trim(),
      country: form.country.trim().toUpperCase() || "US",
      panelUrl: form.panelUrl?.trim() || null,
      panelUser: form.panelUser?.trim() || "admin",
      sshKeyPath: form.sshKeyPath?.trim() || null,
      bastionHost,
      bastionPort: bastionHost ? form.bastionPort || 22 : null,
      bastionUser: bastionHost ? form.bastionUser?.trim() || form.sshUser.trim() : null,
      bastionSshKeyPath: bastionHost ? form.bastionSshKeyPath?.trim() || null : null,
      sshKeyPassphrase: null,
      sslVerify: form.sslVerify,
    };
  };

  const normalizedBastion = (): BastionConfig => ({
    id: selectedBastionId || slug(`${bastionPresetName || form.bastionHost}-${form.bastionHost}`) || crypto.randomUUID(),
    name: bastionPresetName.trim() || form.bastionHost?.trim() || "Bastion",
    host: form.bastionHost?.trim() || "",
    port: form.bastionPort || 22,
    user: form.bastionUser?.trim() || form.sshUser.trim() || "root",
    sshKeyPath: form.bastionSshKeyPath?.trim() || null,
  });

  const applyBastion = (bastionId: string) => {
    setSelectedBastionId(bastionId);
    const bastion = bastions.find((item) => item.id === bastionId);
    if (!bastion) {
      setBastionPresetName("");
      return;
    }

    setBastionPresetName(bastion.name);
    updateForm("bastionHost", bastion.host);
    updateForm("bastionPort", bastion.port);
    updateForm("bastionUser", bastion.user);
    updateForm("bastionSshKeyPath", bastion.sshKeyPath ?? "");
  };

  const saveBastion = async () => {
    setError("");
    const bastion = normalizedBastion();

    if (!bastion.name || !bastion.host || !bastion.user) {
      setError("Bastion name, host and user are required.");
      return;
    }

    try {
      await onSaveBastion(bastion);
      setSelectedBastionId(bastion.id);
      setBastionPresetName(bastion.name);
      setMessage("Bastion saved");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const removeBastion = async () => {
    if (!selectedBastionId) return;

    setError("");
    try {
      await onDeleteBastion(selectedBastionId);
      setSelectedBastionId("");
      setBastionPresetName("");
      setMessage("Bastion deleted");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const installMonitorAgent = async () => {
    setInstallingMonitor(true);
    setError("");
    setMessage("");
    try {
      await invoke<string>("install_monitor_agent");
      setMessage("Monitor agent installed and started");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setInstallingMonitor(false);
    }
  };

  const checkAndInstallUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateStatus("Checking for updates...");
    setError("");
    let update: Awaited<ReturnType<typeof check>> = null;

    try {
      update = await check();
      if (!update) {
        setUpdateStatus("You are on the latest version.");
        return;
      }

      setUpdateStatus(`Update ${update.version} found. Downloading...`);
      let downloaded = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          downloaded = 0;
          setUpdateStatus("Downloading update...");
          return;
        }
        if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setUpdateStatus(`Downloading update... ${Math.round(downloaded / 1024 / 1024)} MB`);
          return;
        }
        setUpdateStatus("Update installed. Restarting...");
      });
      await relaunch();
    } catch (err) {
      setUpdateStatus("");
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setCheckingUpdate(false);
      await update?.close().catch(() => undefined);
    }
  };

  const updateMonitorTarget = async (value: string) => {
    const [kind, id] = value.split(":", 2);
    await onMonitorTargetChange({
      serverId: kind === "server" && id ? id : null,
      bastionId: kind === "bastion" && id ? id : null,
    });
  };

  const detectPanelInfo = async () => {
    const targetServer = selectedServer;
    if (!targetServer) {
      setError("Save the server before detecting 3x-ui.");
      return;
    }

    setError("");
    setMessage("");
    try {
      const info = await invoke<PanelSetupInfo>("get_panel_setup_info", { serverId: targetServer.id });
      const basePath = normalizePanelBasePath(info.webBasePath);
      const panelUrl = `http://${targetServer.host}:${info.port}${basePath}`;
      const updatedServer = { ...targetServer, panelUrl, panelUser: info.username };
      updateForm("panelUrl", panelUrl);
      updateForm("panelUser", info.username);
      await onSaveServer(updatedServer);
      setMessage(info.password ? "3x-ui URL, login and password detected" : "3x-ui URL and login detected. Enter panel password manually.");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const saveServer = async () => {
    setError("");
    const server = normalizedServer();

    if (!server.name || !server.host || !server.sshUser) {
      setError("Name, host and SSH user are required.");
      return;
    }

    try {
      const wasNew = !selectedServer;
      await onSaveServer(server);
      setIsCreatingServer(false);
      setSelectedServerId(server.id);
      setMessage("Server saved");
      if (wasNew) {
        setSetupServerId(server.id);
      }
      if (wasNew && hasMonitor && syncMonitorKey && server.sshKeyPath) {
        try {
          await invoke<string>("sync_monitor_ssh_key", { serverId: server.id });
          setMessage("Server saved and SSH key synced to monitor");
        } catch (err) {
          setError(`Server saved, but monitor key sync failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const testConnection = async () => {
    setTesting(true);
    setError("");
    setMessage("");
    try {
      const server = normalizedServer();
      const result = await invoke<TestConnectionResult>("test_server_connection", {
        server,
        sshPassword: password || null,
        sshKeyPassphrase: keyPassphrase || null,
        bastionPassword: bastionPassword || null,
        panelPassword: panelPassword || null,
      });
      const ping = result.ping.latencyMs === null ? result.ping.message : `${result.ping.latencyMs}ms`;
      const panel = result.panelOk === null ? "" : ` / ${result.panelMessage}`;
      setMessage(`${ping} / ${result.sshMessage}${panel}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  };

  const saveBastionPassword = async () => {
    if (!selectedServer || bastionPassword.length === 0) return;

    await invoke("save_bastion_password", {
      serverId: selectedServer.id,
      password: bastionPassword,
    });
    setBastionPassword("");
    setMessage("Bastion password saved in Keychain");
  };

  const deleteBastionPassword = async () => {
    if (!selectedServer) return;

    await invoke("delete_bastion_password", { serverId: selectedServer.id });
    setBastionPassword("");
    setMessage("Bastion password removed from Keychain");
  };

  const panelInfoSaved = (info: PanelSetupInfo) => {
    const host = setupServer?.host ?? form.host;
    const basePath = normalizePanelBasePath(info.webBasePath);
    const panelUrl = `http://${host}:${info.port}${basePath}`;
    updateForm("panelUrl", panelUrl);
    updateForm("panelUser", info.username);
    if (setupServer) {
      void onSaveServer({ ...setupServer, panelUrl, panelUser: info.username });
    }
  };

  const removeServer = async () => {
    if (!selectedServer) return;
    setError("");
    try {
      await onDeleteServer(selectedServer.id);
      setIsCreatingServer(false);
      setSelectedServerId("");
      setMessage("Server deleted");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const savePassword = async () => {
    if (!selectedServer || password.length === 0) return;

    await invoke("save_ssh_password", {
      serverId: selectedServer.id,
      password,
    });
    setPassword("");
    setMessage("SSH password saved in Keychain");
  };

  const deletePassword = async () => {
    if (!selectedServer) return;

    await invoke("delete_ssh_password", { serverId: selectedServer.id });
    setPassword("");
    setMessage("SSH password removed from Keychain");
  };

  const saveKeyPassphrase = async () => {
    if (!selectedServer || keyPassphrase.length === 0) return;

    await invoke("save_ssh_key_passphrase", {
      serverId: selectedServer.id,
      passphrase: keyPassphrase,
    });
    setKeyPassphrase("");
    setMessage("SSH key passphrase saved in Keychain");
  };

  const deleteKeyPassphrase = async () => {
    if (!selectedServer) return;

    await invoke("delete_ssh_key_passphrase", { serverId: selectedServer.id });
    setKeyPassphrase("");
    setMessage("SSH key passphrase removed from Keychain");
  };

  const savePanelPassword = async () => {
    if (!selectedServer || panelPassword.length === 0) return;

    await invoke("save_three_x_ui_password", {
      serverId: selectedServer.id,
      username: form.panelUser || "admin",
      password: panelPassword,
    });
    setPanelPassword("");
    setMessage("3x-ui password saved in Keychain");
  };

  const deletePanelPassword = async () => {
    if (!selectedServer) return;

    await invoke("delete_three_x_ui_password", { serverId: selectedServer.id });
    setPanelPassword("");
    setMessage("3x-ui password removed from Keychain");
  };

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">Preferences</p>
          <h2>Settings</h2>
          <span className="server-target">{configPath || "~/Library/Application Support/NodeNet/config.json"}</span>
        </div>
        <button
          className="command-button"
          onClick={() => {
            setIsCreatingServer(true);
            setSelectedServerId("");
            setForm(emptyServer());
            setSelectedBastionId("");
            setBastionPresetName("");
            setSyncMonitorKey(true);
            setPassword("");
            setKeyPassphrase("");
            setBastionPassword("");
            setPanelPassword("");
            setMessage("");
            setError("");
            setSetupServerId(null);
          }}
        >
          <Plus size={16} />
          <span>New server</span>
        </button>
      </header>

      {error ? <div className="error-state compact">{error}</div> : null}
      {message ? <p className="settings-message">{message}</p> : null}

      <section className="settings-grid">
        <article className="settings-panel">
          <div className="settings-panel-header">
            <SlidersHorizontal size={18} />
            <h3>General</h3>
          </div>
          <label className="field">
            <span>Polling interval, sec</span>
            <input
              type="number"
              min={2}
              max={120}
              value={pollIntervalSec}
              onChange={(event) => void onPollIntervalChange(Number(event.target.value))}
            />
          </label>
          <label className="field">
            <span>Theme</span>
            <select value={theme} onChange={(event) => void onThemeChange(event.target.value as AppTheme)}>
              <option value="dark">Dark</option>
              <option value="purple-dark">Purple dark</option>
              <option value="green-dark">Green dark</option>
              <option value="full-dark">Full dark</option>
              <option value="contrast">Contrast</option>
              <option value="system">System</option>
            </select>
          </label>
          <label className="field">
            <span>Monitor server</span>
            <select
              value={monitorTargetValue}
              onChange={(event) => void updateMonitorTarget(event.target.value)}
            >
              <option value="">This app</option>
              {servers.map((server) => (
                <option key={server.id} value={`server:${server.id}`}>
                  Server · {server.name}
                </option>
              ))}
              {bastions.map((bastion) => (
                <option key={bastion.id} value={`bastion:${bastion.id}`}>
                  Bastion · {bastion.name}
                </option>
              ))}
            </select>
          </label>
          <div className="settings-actions">
            <button
              className="command-button"
              disabled={!hasMonitor || installingMonitor}
              onClick={() => void installMonitorAgent()}
            >
              <ServerCog size={16} className={installingMonitor ? "spin" : ""} />
              <span>{installingMonitor ? "Installing" : "Install monitor"}</span>
            </button>
          </div>
        </article>

        <article className="settings-panel">
          <div className="settings-panel-header">
            <Download size={18} />
            <h3>Application</h3>
          </div>
          <div className="version-row">
            <span>Current version</span>
            <strong>{appVersion ? `v${appVersion}` : "Detecting..."}</strong>
          </div>
          {updateStatus ? <p className="settings-hint">{updateStatus}</p> : null}
          <div className="settings-actions">
            <button
              className="command-button primary"
              disabled={checkingUpdate}
              onClick={() => void checkAndInstallUpdate()}
            >
              <RefreshCw size={16} className={checkingUpdate ? "spin" : ""} />
              <span>{checkingUpdate ? "Checking" : "Check & update"}</span>
            </button>
          </div>
        </article>

        <article className="settings-panel wide">
          <div className="settings-panel-header split">
            <div>
              <KeyRound size={18} />
              <h3>Server</h3>
            </div>
            <select
              className="compact-select"
              value={selectedServerId}
              onChange={(event) => {
                setIsCreatingServer(event.target.value === "");
                setSelectedServerId(event.target.value);
              }}
            >
              <option value="">New server</option>
              {servers.map((server) => (
                <option key={server.id} value={server.id}>
                  {server.name}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-form-grid">
            <label className="field">
              <span>ID</span>
              <input value={form.id} onChange={(event) => updateForm("id", event.target.value)} placeholder="auto" />
            </label>
            <label className="field">
              <span>Name</span>
              <input value={form.name} onChange={(event) => updateForm("name", event.target.value)} placeholder="Germany 1" />
            </label>
            <label className="field">
              <span>Country</span>
              <input maxLength={2} value={form.country} onChange={(event) => updateForm("country", event.target.value)} placeholder="DE" />
            </label>
            <label className="field">
              <span>Host</span>
              <input value={form.host} onChange={(event) => updateForm("host", event.target.value)} placeholder="1.2.3.4" />
            </label>
            <label className="field">
              <span>SSH port</span>
              <input type="number" min={1} max={65535} value={form.sshPort} onChange={(event) => updateForm("sshPort", Number(event.target.value))} placeholder="22" />
            </label>
            <label className="field">
              <span>SSH user</span>
              <input value={form.sshUser} onChange={(event) => updateForm("sshUser", event.target.value)} placeholder="root" />
            </label>
            <label className="field wide">
              <span>SSH key path</span>
              <input value={form.sshKeyPath ?? ""} onChange={(event) => updateForm("sshKeyPath", event.target.value)} placeholder="~/.ssh/server.pem" />
            </label>
            <label className="field checkbox-field">
              <span>Sync key to monitor</span>
              <input
                type="checkbox"
                checked={syncMonitorKey}
                disabled={!isCreatingServer || !hasMonitor || !form.sshKeyPath?.trim()}
                onChange={(event) => setSyncMonitorKey(event.target.checked)}
              />
            </label>
            <label className="field checkbox-field">
              <span>Verify SSL</span>
              <input type="checkbox" checked={form.sslVerify} onChange={(event) => updateForm("sslVerify", event.target.checked)} />
            </label>
            <label className="field wide">
              <span>3x-ui panel URL</span>
              <input value={form.panelUrl ?? ""} onChange={(event) => updateForm("panelUrl", event.target.value)} placeholder="https://panel.example.com" />
            </label>
            <label className="field">
              <span>3x-ui user</span>
              <input value={form.panelUser ?? ""} onChange={(event) => updateForm("panelUser", event.target.value)} placeholder="admin" />
            </label>
          </div>

          <details className="settings-subsection">
            <summary>Bastion / Jump Host</summary>
            <div className="settings-form-grid">
              <label className="field checkbox-field">
                <span>Connect via bastion</span>
                <input
                  type="checkbox"
                  checked={Boolean(form.bastionHost)}
                  onChange={(event) => {
                    updateForm("bastionHost", event.target.checked ? form.bastionHost || "" : null);
                    updateForm("bastionPort", event.target.checked ? form.bastionPort || 22 : null);
                    updateForm("bastionUser", event.target.checked ? form.bastionUser || form.sshUser : null);
                    updateForm("bastionSshKeyPath", event.target.checked ? form.bastionSshKeyPath || "" : null);
                  }}
                />
              </label>
              <label className="field">
                <span>Saved bastion</span>
                <select value={selectedBastionId} onChange={(event) => applyBastion(event.target.value)}>
                  <option value="">Custom bastion</option>
                  {bastions.map((bastion) => (
                    <option key={bastion.id} value={bastion.id}>
                      {bastion.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Preset name</span>
                <input value={bastionPresetName} onChange={(event) => setBastionPresetName(event.target.value)} placeholder="Main bastion" />
              </label>
              <label className="field">
                <span>Host</span>
                <input value={form.bastionHost ?? ""} onChange={(event) => updateForm("bastionHost", event.target.value)} placeholder="bastion.example.com" />
              </label>
              <label className="field">
                <span>Port</span>
                <input type="number" min={1} max={65535} value={form.bastionPort ?? 22} onChange={(event) => updateForm("bastionPort", Number(event.target.value))} placeholder="22" />
              </label>
              <label className="field">
                <span>User</span>
                <input value={form.bastionUser ?? ""} onChange={(event) => updateForm("bastionUser", event.target.value)} placeholder="root" />
              </label>
              <label className="field">
                <span>Password / key passphrase</span>
                <input type="password" value={bastionPassword} onChange={(event) => setBastionPassword(event.target.value)} placeholder="Keychain secret" />
              </label>
              <label className="field wide">
                <span>SSH key path</span>
                <input value={form.bastionSshKeyPath ?? ""} onChange={(event) => updateForm("bastionSshKeyPath", event.target.value)} placeholder="~/.ssh/bastion_ed25519" />
              </label>
              <div className="settings-actions">
                <button className="command-button" disabled={!selectedServer} onClick={() => void saveBastionPassword()}>
                  <Save size={16} />
                  <span>Save</span>
                </button>
                <button className="command-button danger" disabled={!selectedServer} onClick={() => void deleteBastionPassword()}>
                  <Trash2 size={16} />
                  <span>Delete</span>
                </button>
                <button className="command-button" disabled={!form.bastionHost?.trim()} onClick={() => void saveBastion()}>
                  <Save size={16} />
                  <span>Save bastion</span>
                </button>
                <button className="command-button danger" disabled={!selectedBastionId} onClick={() => void removeBastion()}>
                  <Trash2 size={16} />
                  <span>Delete bastion</span>
                </button>
              </div>
            </div>
          </details>

          {selectedServer ? (
            <div className="server-detail">
              <strong>Panel</strong>
              <code>{selectedServer.panelUrl ?? "not configured"}</code>
              <span>{selectedServer.panelUser ?? "admin"}</span>
            </div>
          ) : null}

          <div className="settings-actions">
            <button className="command-button primary" onClick={() => void saveServer()}>
              <Save size={16} />
              <span>Save server</span>
            </button>
            <button className="command-button" disabled={testing} onClick={() => void testConnection()}>
              <Wifi size={16} className={testing ? "spin" : ""} />
              <span>{testing ? "Testing" : "Test"}</span>
            </button>
            <button className="command-button" disabled={!selectedServer} onClick={() => void detectPanelInfo()}>
              <RefreshCw size={16} />
              <span>Detect 3x-ui</span>
            </button>
            <button className="command-button danger" disabled={!selectedServer} onClick={() => setConfirmDelete(true)}>
              <Trash2 size={16} />
              <span>Delete server</span>
            </button>
          </div>
        </article>

        {setupServer ? (
          <SetupPresets
            server={setupServer}
            onPanelInfoSaved={panelInfoSaved}
            onServerUpdated={onSaveServer}
            onDone={() => setSetupServerId(null)}
          />
        ) : null}

        <article className="settings-panel">
          <div className="settings-panel-header">
            <KeyRound size={18} />
            <h3>SSH Keychain</h3>
          </div>
          <label className="field">
            <span>Password</span>
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="Keychain secret"
            />
          </label>
          <div className="settings-actions">
            <button className="command-button" disabled={!selectedServer} onClick={() => void savePassword()}>
              <Save size={16} />
              <span>Save</span>
            </button>
            <button className="command-button danger" disabled={!selectedServer} onClick={() => void deletePassword()}>
              <Trash2 size={16} />
              <span>Delete</span>
            </button>
          </div>
        </article>

        <article className="settings-panel">
          <div className="settings-panel-header">
            <KeyRound size={18} />
            <h3>SSH Key Passphrase</h3>
          </div>
          <label className="field">
            <span>Passphrase</span>
            <input
              type="password"
              value={keyPassphrase}
              onChange={(event) => setKeyPassphrase(event.target.value)}
              placeholder="Private key secret"
            />
          </label>
          <div className="settings-actions">
            <button className="command-button" disabled={!selectedServer} onClick={() => void saveKeyPassphrase()}>
              <Save size={16} />
              <span>Save</span>
            </button>
            <button className="command-button danger" disabled={!selectedServer} onClick={() => void deleteKeyPassphrase()}>
              <Trash2 size={16} />
              <span>Delete</span>
            </button>
          </div>
        </article>

        <article className="settings-panel">
          <div className="settings-panel-header">
            <ShieldCheck size={18} />
            <h3>3x-ui Keychain</h3>
          </div>
          <label className="field">
            <span>Password</span>
            <input
              type="password"
              value={panelPassword}
              onChange={(event) => setPanelPassword(event.target.value)}
              placeholder="3x-ui secret"
            />
          </label>
          <div className="settings-actions">
            <button className="command-button" disabled={!selectedServer} onClick={() => void savePanelPassword()}>
              <Save size={16} />
              <span>Save</span>
            </button>
            <button className="command-button danger" disabled={!selectedServer} onClick={() => void deletePanelPassword()}>
              <Trash2 size={16} />
              <span>Delete</span>
            </button>
          </div>
        </article>

        <article className="settings-panel wide">
          <div className="settings-panel-header">
            <RefreshCw size={18} />
            <h3>Servers</h3>
          </div>
          <div className="server-table">
            {servers.map((server) => (
              <button
                key={server.id}
                className={selectedServerId === server.id ? "server-table-row selected" : "server-table-row"}
                onClick={() => {
                  setIsCreatingServer(false);
                  setSelectedServerId(server.id);
                }}
              >
                <strong>{server.name}</strong>
                <span className="country-cell">
                  <CountryFlag country={server.country} />
                </span>
                <code>{server.sshUser}@{server.host}:{server.sshPort}</code>
                <code>{server.panelUrl ?? "--"}</code>
                <span>{server.sshKeyPath ? ".pem" : "password"}</span>
              </button>
            ))}
            {servers.length === 0 ? (
              <div className="empty-state compact">
                <KeyRound size={18} />
                <span>No servers configured</span>
              </div>
            ) : null}
          </div>
        </article>
      </section>
      {confirmDelete && selectedServer ? (
        <ConfirmModal
          title={`Delete ${selectedServer.name}?`}
          message="This server and its saved configuration entry will be removed."
          confirmLabel="Delete server"
          onCancel={() => setConfirmDelete(false)}
          onConfirm={() => {
            setConfirmDelete(false);
            void removeServer();
          }}
        />
      ) : null}
    </main>
  );
}

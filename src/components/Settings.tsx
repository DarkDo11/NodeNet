import { invoke } from "@tauri-apps/api/core";
import {
  KeyRound,
  Plus,
  RefreshCw,
  Save,
  ShieldCheck,
  SlidersHorizontal,
  Wifi,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import ConfirmModal from "./ConfirmModal";
import SetupPresets from "./SetupPresets";
import type { AppTheme, PanelSetupInfo, ServerConfig, TestConnectionResult } from "../types";

interface SettingsProps {
  servers: ServerConfig[];
  pollIntervalSec: number;
  theme: AppTheme;
  onPollIntervalChange: (seconds: number) => Promise<void>;
  onThemeChange: (theme: AppTheme) => Promise<void>;
  onSaveServer: (server: ServerConfig) => Promise<void>;
  onDeleteServer: (serverId: string) => Promise<void>;
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
  sshKeyPassphrase: null,
  sslVerify: false,
});

const slug = (value: string) =>
  value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 36);

export default function Settings({
  servers,
  pollIntervalSec,
  theme,
  onPollIntervalChange,
  onThemeChange,
  onSaveServer,
  onDeleteServer,
}: SettingsProps) {
  const [selectedServerId, setSelectedServerId] = useState<string>("");
  const [form, setForm] = useState<ServerConfig>(emptyServer);
  const [password, setPassword] = useState("");
  const [keyPassphrase, setKeyPassphrase] = useState("");
  const [bastionPassword, setBastionPassword] = useState("");
  const [panelPassword, setPanelPassword] = useState("");
  const [setupServerId, setSetupServerId] = useState<string | null>(null);
  const [configPath, setConfigPath] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [testing, setTesting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [selectedServerId, servers],
  );
  const setupServer = useMemo(
    () => servers.find((server) => server.id === setupServerId) ?? null,
    [setupServerId, servers],
  );

  useEffect(() => {
    if (!selectedServerId && servers[0]) {
      setSelectedServerId(servers[0].id);
    }
  }, [selectedServerId, servers]);

  useEffect(() => {
    setForm(selectedServer ? { ...emptyServer(), ...selectedServer } : emptyServer());
    setPassword("");
    setKeyPassphrase("");
    setBastionPassword("");
    setPanelPassword("");
  }, [selectedServer]);

  useEffect(() => {
    void invoke<string>("get_config_path")
      .then(setConfigPath)
      .catch((err) => setError(err instanceof Error ? err.message : String(err)));
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
      sshKeyPassphrase: null,
      sslVerify: form.sslVerify,
    };
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
      setSelectedServerId(server.id);
      setMessage("Server saved");
      if (wasNew) {
        setSetupServerId(server.id);
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
      const ping = result.ping.latencyMs === null ? "Ping failed" : `${result.ping.latencyMs}ms`;
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
    const panelUrl = `http://${host}:${info.port}`;
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
            setSelectedServerId("");
            setForm(emptyServer());
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
              <option value="contrast">Contrast</option>
              <option value="system">System</option>
            </select>
          </label>
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
              onChange={(event) => setSelectedServerId(event.target.value)}
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
              <input value={form.name} onChange={(event) => updateForm("name", event.target.value)} />
            </label>
            <label className="field">
              <span>Country</span>
              <input maxLength={2} value={form.country} onChange={(event) => updateForm("country", event.target.value)} />
            </label>
            <label className="field">
              <span>Host</span>
              <input value={form.host} onChange={(event) => updateForm("host", event.target.value)} />
            </label>
            <label className="field">
              <span>SSH port</span>
              <input type="number" min={1} max={65535} value={form.sshPort} onChange={(event) => updateForm("sshPort", Number(event.target.value))} />
            </label>
            <label className="field">
              <span>SSH user</span>
              <input value={form.sshUser} onChange={(event) => updateForm("sshUser", event.target.value)} />
            </label>
            <label className="field wide">
              <span>SSH key path</span>
              <input value={form.sshKeyPath ?? ""} onChange={(event) => updateForm("sshKeyPath", event.target.value)} placeholder="~/.ssh/server.pem" />
            </label>
            <label className="field checkbox-field">
              <span>Verify SSL</span>
              <input type="checkbox" checked={form.sslVerify} onChange={(event) => updateForm("sslVerify", event.target.checked)} />
            </label>
            <label className="field wide">
              <span>3x-ui panel URL</span>
              <input value={form.panelUrl ?? ""} onChange={(event) => updateForm("panelUrl", event.target.value)} />
            </label>
            <label className="field">
              <span>3x-ui user</span>
              <input value={form.panelUser ?? ""} onChange={(event) => updateForm("panelUser", event.target.value)} />
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
                  }}
                />
              </label>
              <label className="field">
                <span>Host</span>
                <input value={form.bastionHost ?? ""} onChange={(event) => updateForm("bastionHost", event.target.value)} />
              </label>
              <label className="field">
                <span>Port</span>
                <input type="number" min={1} max={65535} value={form.bastionPort ?? 22} onChange={(event) => updateForm("bastionPort", Number(event.target.value))} />
              </label>
              <label className="field">
                <span>User</span>
                <input value={form.bastionUser ?? ""} onChange={(event) => updateForm("bastionUser", event.target.value)} />
              </label>
              <label className="field">
                <span>Password</span>
                <input type="password" value={bastionPassword} onChange={(event) => setBastionPassword(event.target.value)} placeholder="Keychain secret" />
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
                onClick={() => setSelectedServerId(server.id)}
              >
                <strong>{server.name}</strong>
                <span>{server.country}</span>
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

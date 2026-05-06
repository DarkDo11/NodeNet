import { invoke } from "@tauri-apps/api/core";
import { KeyRound, RefreshCw, Save, ShieldCheck, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ServerConfig } from "../types";

interface SettingsProps {
  servers: ServerConfig[];
  pollIntervalSec: number;
  onPollIntervalChange: (seconds: number) => void;
}

export default function Settings({
  servers,
  pollIntervalSec,
  onPollIntervalChange,
}: SettingsProps) {
  const [selectedServerId, setSelectedServerId] = useState<string>("");
  const [password, setPassword] = useState("");
  const [panelUsername, setPanelUsername] = useState("admin");
  const [panelPassword, setPanelPassword] = useState("");
  const [configPath, setConfigPath] = useState("");
  const [message, setMessage] = useState("");

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? servers[0],
    [selectedServerId, servers],
  );

  useEffect(() => {
    if (!selectedServerId && servers[0]) {
      setSelectedServerId(servers[0].id);
    }
  }, [selectedServerId, servers]);

  useEffect(() => {
    if (selectedServer) {
      setPanelUsername(selectedServer.panelUser || "admin");
    }
  }, [selectedServer]);

  useEffect(() => {
    void invoke<string>("get_config_path")
      .then(setConfigPath)
      .catch((error) => setMessage(error instanceof Error ? error.message : String(error)));
  }, []);

  const savePassword = async () => {
    if (!selectedServer || password.length === 0) return;

    await invoke("save_ssh_password", {
      serverId: selectedServer.id,
      password,
    });
    setPassword("");
    setMessage("Password saved in Keychain");
  };

  const deletePassword = async () => {
    if (!selectedServer) return;

    await invoke("delete_ssh_password", { serverId: selectedServer.id });
    setPassword("");
    setMessage("Password removed from Keychain");
  };

  const savePanelPassword = async () => {
    if (!selectedServer || panelPassword.length === 0) return;

    await invoke("save_three_x_ui_password", {
      serverId: selectedServer.id,
      username: panelUsername,
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
      </header>

      <section className="settings-grid">
        <article className="settings-panel">
          <div className="settings-panel-header">
            <RefreshCw size={18} />
            <h3>Polling</h3>
          </div>
          <label className="field">
            <span>Interval, sec</span>
            <input
              type="number"
              min={2}
              max={120}
              value={pollIntervalSec}
              onChange={(event) => onPollIntervalChange(Number(event.target.value))}
            />
          </label>
        </article>

        <article className="settings-panel">
          <div className="settings-panel-header">
            <KeyRound size={18} />
            <h3>SSH Keychain</h3>
          </div>
          <label className="field">
            <span>Server</span>
            <select
              value={selectedServer?.id ?? ""}
              onChange={(event) => setSelectedServerId(event.target.value)}
            >
              {servers.map((server) => (
                <option key={server.id} value={server.id}>
                  {server.name}
                </option>
              ))}
            </select>
          </label>
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
            <button className="command-button" onClick={() => void savePassword()}>
              <Save size={16} />
              <span>Save</span>
            </button>
            <button className="command-button danger" onClick={() => void deletePassword()}>
              <Trash2 size={16} />
              <span>Delete</span>
            </button>
          </div>
          {message ? <p className="settings-message">{message}</p> : null}
        </article>

        <article className="settings-panel">
          <div className="settings-panel-header">
            <ShieldCheck size={18} />
            <h3>3x-ui Keychain</h3>
          </div>
          <label className="field">
            <span>Server</span>
            <select
              value={selectedServer?.id ?? ""}
              onChange={(event) => setSelectedServerId(event.target.value)}
            >
              {servers.map((server) => (
                <option key={server.id} value={server.id}>
                  {server.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Username</span>
            <input
              value={panelUsername}
              onChange={(event) => setPanelUsername(event.target.value)}
              placeholder="admin"
            />
          </label>
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
            <button className="command-button" onClick={() => void savePanelPassword()}>
              <Save size={16} />
              <span>Save</span>
            </button>
            <button className="command-button danger" onClick={() => void deletePanelPassword()}>
              <Trash2 size={16} />
              <span>Delete</span>
            </button>
          </div>
          {message ? <p className="settings-message">{message}</p> : null}
        </article>

        <article className="settings-panel wide">
          <div className="settings-panel-header">
            <KeyRound size={18} />
            <h3>Servers</h3>
          </div>
          <div className="server-table">
            {servers.map((server) => (
              <div key={server.id} className="server-table-row">
                <strong>{server.name}</strong>
                <span>{server.country}</span>
                <code>{server.sshUser}@{server.host}:{server.sshPort}</code>
                <code>{server.panelUrl ?? "--"}</code>
                <span>{server.sshKeyPath ? ".pem" : "password"}</span>
              </div>
            ))}
          </div>
        </article>
      </section>
    </main>
  );
}

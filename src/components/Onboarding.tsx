import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, KeyRound, Plus, ShieldCheck, Wifi } from "lucide-react";
import { useState } from "react";
import SetupPresets from "./SetupPresets";
import type { BastionConfig, ServerConfig, TestConnectionResult } from "../types";

interface OnboardingProps {
  bastions: BastionConfig[];
  onCreateServer: (server: ServerConfig) => Promise<void>;
  onSaveBastion?: (bastion: BastionConfig) => Promise<void>;
  onSetupStarted?: (serverId: string) => void;
  onFinishSetup?: () => void;
}

const makeId = (name: string, host: string) => {
  const base = `${name}-${host}`
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 34);
  return base || crypto.randomUUID();
};

const makeBastionId = (name: string, host: string) => makeId(name || "bastion", host);

export default function Onboarding({
  bastions,
  onCreateServer,
  onSaveBastion,
  onSetupStarted,
  onFinishSetup,
}: OnboardingProps) {
  const [name, setName] = useState("Germany 1");
  const [host, setHost] = useState("");
  const [sshPort, setSshPort] = useState(22);
  const [sshUser, setSshUser] = useState("root");
  const [country, setCountry] = useState("DE");
  const [panelUrl, setPanelUrl] = useState("");
  const [panelUser, setPanelUser] = useState("admin");
  const [sslVerify, setSslVerify] = useState(false);
  const [sshPassword, setSshPassword] = useState("");
  const [sshKeyPath, setSshKeyPath] = useState("");
  const [sshKeyPassphrase, setSshKeyPassphrase] = useState("");
  const [useBastion, setUseBastion] = useState(false);
  const [bastionHost, setBastionHost] = useState("");
  const [bastionPort, setBastionPort] = useState(22);
  const [bastionUser, setBastionUser] = useState("root");
  const [bastionPassword, setBastionPassword] = useState("");
  const [bastionSshKeyPath, setBastionSshKeyPath] = useState("");
  const [selectedBastionId, setSelectedBastionId] = useState("");
  const [bastionPresetName, setBastionPresetName] = useState("");
  const [saveBastionPreset, setSaveBastionPreset] = useState(false);
  const [panelPassword, setPanelPassword] = useState("");
  const [error, setError] = useState("");
  const [testMessage, setTestMessage] = useState("");
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [createdServer, setCreatedServer] = useState<ServerConfig | null>(null);

  const buildServer = (): ServerConfig => ({
    id: makeId(name, host),
    name: name.trim(),
    host: host.trim(),
    sshPort,
    sshUser: sshUser.trim(),
    country: country.trim().toUpperCase() || "US",
    panelUrl: panelUrl.trim() || null,
    panelUser: panelUser.trim() || "admin",
    sshKeyPath: sshKeyPath.trim() || null,
    bastionHost: useBastion ? bastionHost.trim() || null : null,
    bastionPort: useBastion ? bastionPort || 22 : null,
    bastionUser: useBastion ? bastionUser.trim() || sshUser.trim() : null,
    bastionSshKeyPath: useBastion ? bastionSshKeyPath.trim() || null : null,
    sshKeyPassphrase: null,
    sslVerify,
  });

  const buildBastion = (): BastionConfig => ({
    id: selectedBastionId || makeBastionId(bastionPresetName || bastionHost, bastionHost),
    name: bastionPresetName.trim() || bastionHost.trim(),
    host: bastionHost.trim(),
    port: bastionPort || 22,
    user: bastionUser.trim() || sshUser.trim() || "root",
    sshKeyPath: bastionSshKeyPath.trim() || null,
  });

  const applyBastion = (bastionId: string) => {
    setSelectedBastionId(bastionId);
    const bastion = bastions.find((item) => item.id === bastionId);
    if (!bastion) {
      setBastionPresetName("");
      return;
    }

    setUseBastion(true);
    setBastionPresetName(bastion.name);
    setBastionHost(bastion.host);
    setBastionPort(bastion.port);
    setBastionUser(bastion.user);
    setBastionSshKeyPath(bastion.sshKeyPath ?? "");
  };

  const submit = async () => {
    if (!name.trim() || !host.trim() || !sshUser.trim()) {
      setError("Name, host and SSH user are required.");
      return;
    }

    const server = buildServer();

    setSaving(true);
    setError("");
    try {
      onSetupStarted?.(server.id);
      await onCreateServer(server);
      if (sshPassword) {
        await invoke("save_ssh_password", { serverId: server.id, password: sshPassword });
      }
      if (sshKeyPassphrase) {
        await invoke("save_ssh_key_passphrase", { serverId: server.id, passphrase: sshKeyPassphrase });
      }
      if (bastionPassword) {
        await invoke("save_bastion_password", { serverId: server.id, password: bastionPassword });
      }
      if (panelPassword) {
        await invoke("save_three_x_ui_password", {
          serverId: server.id,
          username: server.panelUser || "admin",
          password: panelPassword,
        });
      }
      if (useBastion && saveBastionPreset && onSaveBastion) {
        const bastion = buildBastion();
        if (bastion.name && bastion.host) {
          await onSaveBastion(bastion);
          setSelectedBastionId(bastion.id);
          setBastionPresetName(bastion.name);
        }
      }
      setCreatedServer(server);
    } catch (err) {
      onFinishSetup?.();
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const testConnection = async () => {
    const server = buildServer();

    setTesting(true);
    setError("");
    setTestMessage("");
    try {
      const result = await invoke<TestConnectionResult>("test_server_connection", {
        server,
        sshPassword: sshPassword || null,
        sshKeyPassphrase: sshKeyPassphrase || null,
        bastionPassword: bastionPassword || null,
        panelPassword: panelPassword || null,
      });
      const ping = result.ping.latencyMs === null ? result.ping.message : `${result.ping.latencyMs}ms`;
      const panel = result.panelOk === null ? "" : ` / ${result.panelMessage}`;
      setTestMessage(`${ping} / ${result.sshMessage}${panel}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  };

  if (createdServer) {
    return (
      <main className="onboarding-screen">
        <section className="onboarding-panel">
          <div>
            <p className="eyebrow">First launch</p>
            <h2>Setup presets</h2>
            <span className="server-target">{createdServer.name}</span>
          </div>
          <SetupPresets
            server={createdServer}
            onServerUpdated={async (server) => {
              await onCreateServer(server);
              setCreatedServer(server);
            }}
            onDone={onFinishSetup}
          />
          <div className="settings-actions">
            <button className="command-button" onClick={() => setCreatedServer(null)}>
              <ArrowLeft size={16} />
              <span>Back</span>
            </button>
            <button className="command-button" onClick={onFinishSetup}>
              <span>Skip presets</span>
            </button>
          </div>
        </section>
      </main>
    );
  }

  return (
    <main className="onboarding-screen">
      <section className="onboarding-panel">
        <div className="brand-mark onboarding-mark">
          <ShieldCheck size={24} />
        </div>
        <div>
          <p className="eyebrow">First launch</p>
          <h2>Add your first server</h2>
          <span className="server-target">NodeNet workspace</span>
        </div>

        {error ? <div className="error-state compact">{error}</div> : null}
        {testMessage ? <p className="settings-message">{testMessage}</p> : null}

        <div className="settings-form-grid">
          <label className="field">
            <span>Name</span>
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Germany 1" />
          </label>
          <label className="field">
            <span>Country</span>
            <input value={country} onChange={(event) => setCountry(event.target.value)} maxLength={2} placeholder="DE" />
          </label>
          <label className="field wide">
            <span>Host</span>
            <input value={host} onChange={(event) => setHost(event.target.value)} placeholder="1.2.3.4" />
          </label>
          <label className="field">
            <span>SSH user</span>
            <input value={sshUser} onChange={(event) => setSshUser(event.target.value)} placeholder="root" />
          </label>
          <label className="field">
            <span>SSH port</span>
            <input type="number" min={1} max={65535} value={sshPort} onChange={(event) => setSshPort(Number(event.target.value))} placeholder="22" />
          </label>
          <label className="field wide">
            <span>SSH password</span>
            <input type="password" value={sshPassword} onChange={(event) => setSshPassword(event.target.value)} placeholder="Keychain secret" />
          </label>
          <label className="field wide">
            <span>SSH key path</span>
            <input value={sshKeyPath} onChange={(event) => setSshKeyPath(event.target.value)} placeholder="~/.ssh/id_ed25519" />
          </label>
          <label className="field">
            <span>SSH key passphrase</span>
            <input type="password" value={sshKeyPassphrase} onChange={(event) => setSshKeyPassphrase(event.target.value)} placeholder="Keychain secret" />
          </label>
          <label className="field wide">
            <span>3x-ui panel URL</span>
            <input value={panelUrl} onChange={(event) => setPanelUrl(event.target.value)} placeholder="https://panel.example.com" />
          </label>
          <label className="field">
            <span>Panel user</span>
            <input value={panelUser} onChange={(event) => setPanelUser(event.target.value)} placeholder="admin" />
          </label>
          <label className="field">
            <span>Panel password</span>
            <input type="password" value={panelPassword} onChange={(event) => setPanelPassword(event.target.value)} placeholder="3x-ui secret" />
          </label>
          <label className="field checkbox-field">
            <span>Verify SSL</span>
            <input type="checkbox" checked={sslVerify} onChange={(event) => setSslVerify(event.target.checked)} />
          </label>
        </div>

        <details className="settings-subsection">
          <summary>Bastion / Jump Host</summary>
          <div className="settings-form-grid">
            <label className="field checkbox-field">
              <span>Connect via bastion</span>
              <input type="checkbox" checked={useBastion} onChange={(event) => setUseBastion(event.target.checked)} />
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
              <input value={bastionHost} onChange={(event) => setBastionHost(event.target.value)} placeholder="bastion.example.com" />
            </label>
            <label className="field">
              <span>Port</span>
              <input type="number" min={1} max={65535} value={bastionPort} onChange={(event) => setBastionPort(Number(event.target.value))} />
            </label>
            <label className="field">
              <span>User</span>
              <input value={bastionUser} onChange={(event) => setBastionUser(event.target.value)} placeholder="root" />
            </label>
            <label className="field wide">
              <span>Bastion password / key passphrase</span>
              <input type="password" value={bastionPassword} onChange={(event) => setBastionPassword(event.target.value)} placeholder="Keychain secret" />
            </label>
            <label className="field wide">
              <span>Bastion SSH key path</span>
              <input value={bastionSshKeyPath} onChange={(event) => setBastionSshKeyPath(event.target.value)} placeholder="~/.ssh/bastion_ed25519" />
            </label>
            <label className="field checkbox-field">
              <span>Save bastion</span>
              <input type="checkbox" checked={saveBastionPreset} onChange={(event) => setSaveBastionPreset(event.target.checked)} />
            </label>
          </div>
        </details>

        <div className="settings-actions">
          <button className="command-button" disabled={testing || !host.trim()} onClick={() => void testConnection()}>
            <Wifi size={16} className={testing ? "spin" : ""} />
            <span>{testing ? "Testing" : "Test"}</span>
          </button>
          <button className="command-button primary" disabled={saving} onClick={() => void submit()}>
            {saving ? <KeyRound size={16} className="spin" /> : <Plus size={16} />}
            <span>{saving ? "Saving" : "Continue to presets"}</span>
          </button>
        </div>
      </section>
    </main>
  );
}

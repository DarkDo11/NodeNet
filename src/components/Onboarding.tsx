import { invoke } from "@tauri-apps/api/core";
import { KeyRound, Plus, ShieldCheck } from "lucide-react";
import { useState } from "react";
import type { ServerConfig } from "../types";

interface OnboardingProps {
  onCreateServer: (server: ServerConfig) => Promise<void>;
}

const makeId = (name: string, host: string) => {
  const base = `${name}-${host}`
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 34);
  return base || crypto.randomUUID();
};

export default function Onboarding({ onCreateServer }: OnboardingProps) {
  const [name, setName] = useState("Germany 1");
  const [host, setHost] = useState("");
  const [sshPort, setSshPort] = useState(22);
  const [sshUser, setSshUser] = useState("root");
  const [country, setCountry] = useState("DE");
  const [panelUrl, setPanelUrl] = useState("");
  const [panelUser, setPanelUser] = useState("admin");
  const [sshPassword, setSshPassword] = useState("");
  const [panelPassword, setPanelPassword] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  const submit = async () => {
    if (!name.trim() || !host.trim() || !sshUser.trim()) {
      setError("Name, host and SSH user are required.");
      return;
    }

    const server: ServerConfig = {
      id: makeId(name, host),
      name: name.trim(),
      host: host.trim(),
      sshPort,
      sshUser: sshUser.trim(),
      country: country.trim().toUpperCase() || "US",
      panelUrl: panelUrl.trim() || null,
      panelUser: panelUser.trim() || "admin",
      sshKeyPath: null,
    };

    setSaving(true);
    setError("");
    try {
      await onCreateServer(server);
      if (sshPassword) {
        await invoke("save_ssh_password", { serverId: server.id, password: sshPassword });
      }
      if (panelPassword) {
        await invoke("save_three_x_ui_password", {
          serverId: server.id,
          username: server.panelUser || "admin",
          password: panelPassword,
        });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

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

        <div className="settings-form-grid">
          <label className="field">
            <span>Name</span>
            <input value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <label className="field">
            <span>Country</span>
            <input value={country} onChange={(event) => setCountry(event.target.value)} maxLength={2} />
          </label>
          <label className="field wide">
            <span>Host</span>
            <input value={host} onChange={(event) => setHost(event.target.value)} placeholder="1.2.3.4" />
          </label>
          <label className="field">
            <span>SSH user</span>
            <input value={sshUser} onChange={(event) => setSshUser(event.target.value)} />
          </label>
          <label className="field">
            <span>SSH port</span>
            <input type="number" min={1} max={65535} value={sshPort} onChange={(event) => setSshPort(Number(event.target.value))} />
          </label>
          <label className="field wide">
            <span>SSH password</span>
            <input type="password" value={sshPassword} onChange={(event) => setSshPassword(event.target.value)} />
          </label>
          <label className="field wide">
            <span>3x-ui panel URL</span>
            <input value={panelUrl} onChange={(event) => setPanelUrl(event.target.value)} placeholder="https://panel.example.com" />
          </label>
          <label className="field">
            <span>Panel user</span>
            <input value={panelUser} onChange={(event) => setPanelUser(event.target.value)} />
          </label>
          <label className="field">
            <span>Panel password</span>
            <input type="password" value={panelPassword} onChange={(event) => setPanelPassword(event.target.value)} />
          </label>
        </div>

        <button className="command-button primary" disabled={saving} onClick={() => void submit()}>
          {saving ? <KeyRound size={16} className="spin" /> : <Plus size={16} />}
          <span>{saving ? "Saving" : "Create server"}</span>
        </button>
      </section>
    </main>
  );
}

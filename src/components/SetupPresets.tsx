import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, KeyRound, Play, Plus, RefreshCw, TerminalSquare } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { PanelSetupInfo, ServerConfig } from "../types";
import CommandOutputModal from "./CommandOutputModal";

interface SetupPresetsProps {
  server: ServerConfig;
  onPanelInfoSaved?: (info: PanelSetupInfo) => void;
  onServerUpdated?: (server: ServerConfig) => void | Promise<void>;
  onDone?: () => void;
}

interface SshKeyPair {
  privateKeyPath: string;
  publicKeyPath: string;
}

type PresetId =
  | "install3xui"
  | "ipReputation"
  | "bbr"
  | "benchmark"
  | "region"
  | "hardenSsh"
  | "ufw"
  | "sshKey";

interface PresetItem {
  id: PresetId;
  name: string;
  description: string;
  command: string;
  recommended: boolean;
  outputWindow?: boolean;
}

const presets: PresetItem[] = [
  {
    id: "install3xui",
    name: "Install 3x-ui panel",
    description: "Panel will be configured on port 65333.",
    command: "bash <(curl -Ls https://raw.githubusercontent.com/mhsanaei/3x-ui/master/install.sh)",
    recommended: true,
  },
  {
    id: "ipReputation",
    name: "Check IP reputation",
    description: "Runs IP.Check.Place and opens the command output.",
    command: "bash <(curl -Ls https://IP.Check.Place) -l en",
    recommended: true,
    outputWindow: true,
  },
  {
    id: "bbr",
    name: "Enable BBR congestion control",
    description: "Enables fq queueing and BBR TCP congestion control.",
    command:
      'echo "net.core.default_qdisc=fq" >> /etc/sysctl.conf && echo "net.ipv4.tcp_congestion_control=bbr" >> /etc/sysctl.conf && sysctl -p',
    recommended: true,
  },
  {
    id: "benchmark",
    name: "Benchmark server speed",
    description: "Runs bench.sh and opens the command output.",
    command: "bash <(curl -Ls https://bench.sh)",
    recommended: false,
    outputWindow: true,
  },
  {
    id: "region",
    name: "Geo/IP region test",
    description: "Runs ipregion.sh and opens the command output.",
    command: "bash <(wget -qO- https://github.com/Davoyan/ipregion/raw/main/ipregion.sh)",
    recommended: false,
    outputWindow: true,
  },
  {
    id: "hardenSsh",
    name: "Harden SSH",
    description: "Disables password auth, enables public key auth, and restarts SSH.",
    command:
      "sed -i 's/#PasswordAuthentication yes/PasswordAuthentication no/' /etc/ssh/sshd_config && sed -i 's/PasswordAuthentication yes/PasswordAuthentication no/' /etc/ssh/sshd_config && sed -i 's/#PubkeyAuthentication yes/PubkeyAuthentication yes/' /etc/ssh/sshd_config && systemctl restart ssh",
    recommended: true,
  },
  {
    id: "ufw",
    name: "Configure UFW firewall",
    description: "Restricts SSH to your management IP, opens panel/HTTPS ports, and shows status.",
    command: "",
    recommended: true,
    outputWindow: true,
  },
  {
    id: "sshKey",
    name: "Copy SSH public key",
    description: "Adds a selected ~/.ssh public key to authorized_keys on the server.",
    command: "",
    recommended: true,
  },
];

export default function SetupPresets({ server, onPanelInfoSaved, onServerUpdated, onDone }: SetupPresetsProps) {
  const [selected, setSelected] = useState<Record<PresetId, boolean>>(() =>
    Object.fromEntries(presets.map((preset) => [preset.id, preset.recommended])) as Record<PresetId, boolean>,
  );
  const [running, setRunning] = useState<PresetId | "all" | null>(null);
  const [completed, setCompleted] = useState<Partial<Record<PresetId, boolean>>>({});
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [streamingOutput, setStreamingOutput] = useState<{
    title: string;
    command: string;
    resolve: () => void;
    reject: (error: Error) => void;
    error: string | null;
  } | null>(null);
  const [managementIp, setManagementIp] = useState("");
  const [keyPaths, setKeyPaths] = useState<string[]>([]);
  const [selectedKeyPath, setSelectedKeyPath] = useState("");
  const [panelUsername, setPanelUsername] = useState(server.panelUser ?? "admin");
  const [panelPassword, setPanelPassword] = useState("");
  const [showPanelCredentialPrompt, setShowPanelCredentialPrompt] = useState(false);
  const [creatingKey, setCreatingKey] = useState(false);
  const [newKeyName, setNewKeyName] = useState(`nodenet_${server.id}_ed25519`);

  const selectedCount = useMemo(
    () => presets.filter((preset) => selected[preset.id]).length,
    [selected],
  );

  useEffect(() => {
    void fetch("https://api.ipify.org")
      .then((response) => response.text())
      .then((ip) => setManagementIp(ip.trim()))
      .catch(() => undefined);
    void invoke<string[]>("list_ssh_public_keys")
      .then((paths) => {
        setKeyPaths(paths);
        setSelectedKeyPath(paths[0] ?? "");
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    setNewKeyName(`nodenet_${server.id}_ed25519`);
  }, [server.id]);

  const runPreset = async (preset: PresetItem) => {
    setError("");
    setMessage("");
    setRunning(preset.id);
    try {
      const command = await commandForPreset(preset);
      if (preset.outputWindow) {
        await new Promise<void>((resolve, reject) => {
          setStreamingOutput({
            title: preset.name,
            command,
            resolve,
            reject,
            error: null,
          });
        });
      } else {
        await invoke<string>("run_preset_command", {
          serverId: server.id,
          command,
        });
      }
      setCompleted((current) => ({ ...current, [preset.id]: true }));
      setMessage(`${preset.name} finished`);
      if (preset.id === "install3xui") {
        setShowPanelCredentialPrompt(true);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setRunning(null);
    }
  };

  const runSelected = async () => {
    setRunning("all");
    setError("");
    setMessage("");
    try {
      for (const preset of presets) {
        if (selected[preset.id]) {
          await runPreset(preset);
        }
      }
      if (selected.install3xui) {
        await fetchPanelSetupInfo();
      }
      setMessage("Selected setup presets finished");
      onDone?.();
    } finally {
      setRunning(null);
    }
  };

  const commandForPreset = async (preset: PresetItem) => {
    if (preset.id === "ufw") {
      if (!managementIp.trim()) {
        throw new Error("Your management IP is required before configuring UFW.");
      }
      const ip = shellQuote(managementIp.trim());
      return [
        `ufw allow from ${ip} to any port 22`,
        "ufw delete allow 22/tcp || true",
        "ufw allow 65333/tcp",
        "ufw allow 443/tcp",
        "yes | ufw enable",
        "ufw reload",
        "ufw status verbose",
      ].join(" && ");
    }

    if (preset.id === "sshKey") {
      if (!selectedKeyPath) {
        throw new Error("Select an SSH public key first.");
      }
      const publicKey = await invoke<string>("read_ssh_public_key", { path: selectedKeyPath });
      return `mkdir -p ~/.ssh && chmod 700 ~/.ssh && grep -qxF ${shellQuote(publicKey.trim())} ~/.ssh/authorized_keys 2>/dev/null || echo ${shellQuote(publicKey.trim())} >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys`;
    }

    return preset.command;
  };

  const fetchPanelSetupInfo = async () => {
    const info = await invoke<PanelSetupInfo>("get_panel_setup_info", { serverId: server.id });
    setPanelUsername(info.username);
    setPanelPassword(info.password);
    onPanelInfoSaved?.(info);
    if (info.source === "default") {
      setError("3x-ui credentials were not found automatically. Enter the panel login manually.");
      setShowPanelCredentialPrompt(true);
      return;
    }
    if (info.password) {
      setMessage(`3x-ui panel saved from ${info.source}: ${info.username} on port ${info.port}`);
    }
    setShowPanelCredentialPrompt(!info.password);
  };

  const savePanelCredentials = async () => {
    if (!panelPassword) return;
    await invoke("save_three_x_ui_password", {
      serverId: server.id,
      username: panelUsername || "admin",
      password: panelPassword,
    });
    setPanelPassword("");
    setShowPanelCredentialPrompt(false);
    setMessage("3x-ui credentials saved in Keychain");
  };

  const createAndLoadSshKey = async () => {
    setCreatingKey(true);
    setError("");
    setMessage("");
    try {
      const keyPair = await invoke<SshKeyPair>("create_ssh_key_pair", {
        serverId: server.id,
        keyName: newKeyName,
      });
      setKeyPaths((current) => Array.from(new Set([keyPair.publicKeyPath, ...current])).sort());
      setSelectedKeyPath(keyPair.publicKeyPath);
      const updatedServer = { ...server, sshKeyPath: keyPair.privateKeyPath };
      if (onServerUpdated) {
        await onServerUpdated(updatedServer);
      } else {
        await invoke("upsert_server", { server: updatedServer });
      }
      setMessage(`SSH key created and loaded: ${keyPair.privateKeyPath}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setCreatingKey(false);
    }
  };

  return (
    <article className="settings-panel wide setup-presets">
      <div className="settings-panel-header split">
        <div>
          <TerminalSquare size={18} />
          <h3>Setup presets</h3>
        </div>
        <button className="command-button primary" disabled={running !== null || selectedCount === 0} onClick={() => void runSelected()}>
          {running === "all" ? <RefreshCw size={16} className="spin" /> : <Play size={16} />}
          <span>{running === "all" ? "Running" : `Run selected (${selectedCount})`}</span>
        </button>
      </div>

      {error ? <div className="error-state compact">{error}</div> : null}
      {message ? <p className="settings-message">{message}</p> : null}

      <div className="preset-list">
        {presets.map((preset) => (
          <div className="preset-row" key={preset.id}>
            <label className="preset-check">
              <input
                type="checkbox"
                checked={selected[preset.id]}
                onChange={(event) =>
                  setSelected((current) => ({ ...current, [preset.id]: event.target.checked }))
                }
              />
              <span>
                <strong>{preset.name}</strong>
                <small>{preset.description}</small>
              </span>
            </label>
            {completed[preset.id] ? <CheckCircle2 size={16} className="preset-done" /> : null}
            <button className="command-button" disabled={running !== null} onClick={() => void runPreset(preset)}>
              {running === preset.id ? <RefreshCw size={16} className="spin" /> : <Play size={16} />}
              <span>{running === preset.id ? "Running" : "Run"}</span>
            </button>
          </div>
        ))}
      </div>

      <div className="settings-form-grid">
        <label className="field">
          <span>Your management IP</span>
          <input value={managementIp} onChange={(event) => setManagementIp(event.target.value)} />
        </label>
        <label className="field wide">
          <span>SSH public key</span>
          <select value={selectedKeyPath} onChange={(event) => setSelectedKeyPath(event.target.value)}>
            <option value="">Select key</option>
            {keyPaths.map((path) => (
              <option value={path} key={path}>
                {path}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>New key name</span>
          <input value={newKeyName} onChange={(event) => setNewKeyName(event.target.value)} placeholder="nodenet_server_ed25519" />
        </label>
        <div className="settings-actions preset-key-actions">
          <button className="command-button" disabled={creatingKey || running !== null || !newKeyName.trim()} onClick={() => void createAndLoadSshKey()}>
            {creatingKey ? <RefreshCw size={16} className="spin" /> : <Plus size={16} />}
            <span>{creatingKey ? "Creating" : "Create and load new SSH key"}</span>
          </button>
        </div>
      </div>

      {showPanelCredentialPrompt ? (
        <div className="settings-form-grid">
          <label className="field">
            <span>3x-ui user</span>
            <input value={panelUsername} onChange={(event) => setPanelUsername(event.target.value)} />
          </label>
          <label className="field">
            <span>3x-ui password</span>
            <input type="password" value={panelPassword} onChange={(event) => setPanelPassword(event.target.value)} />
          </label>
          <div className="settings-actions">
            <button className="command-button" disabled={!panelPassword} onClick={() => void savePanelCredentials()}>
              <KeyRound size={16} />
              <span>Save panel login</span>
            </button>
          </div>
        </div>
      ) : null}

      {streamingOutput ? (
        <CommandOutputModal
          title={streamingOutput.title}
          serverId={server.id}
          command={streamingOutput.command}
          onComplete={(nextError) =>
            setStreamingOutput((current) =>
              current ? { ...current, error: nextError } : current,
            )
          }
          onClose={() => {
            const current = streamingOutput;
            setStreamingOutput(null);
            if (current.error) {
              current.reject(new Error(current.error));
            } else {
              current.resolve();
            }
          }}
        />
      ) : null}
    </article>
  );
}

const shellQuote = (value: string) => `'${value.replace(/'/g, "'\\''")}'`;

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
  | "sshKey"
  | "ipReputation"
  | "bbr"
  | "benchmark"
  | "region"
  | "hardenSsh"
  | "ufw"
  | "panelSsl";

interface PresetItem {
  id: PresetId;
  name: string;
  description: string;
  command: string;
  recommended: boolean;
  outputWindow?: boolean;
}

// A panel certificate is only worth having if it renews itself, and both ACME
// clients renew unattended out of cron - certbot from its timer, acme.sh from
// its own crontab entry. Neither can pass an HTTP-01 challenge on a server set
// up the way these are: ufw keeps :80 shut, and nginx usually holds the port
// anyway. So the renewal has to open the door itself and put everything back
// afterwards. One helper owns that dance and both clients call it, rather than
// each carrying its own half-correct copy.
const acmeHelperPath = "/usr/local/sbin/nodenet-acme-port80";

const acmePortHelper = [
  "#!/bin/sh",
  "# Managed by NodeNet. Frees and reopens :80 around an ACME HTTP-01 challenge.",
  // cron runs hooks with a bare PATH (/usr/bin:/bin on Debian and Ubuntu) while
  // ufw lives in /usr/sbin, so an unqualified `ufw` is simply not found there:
  // the port never opens and the renewal dies with a challenge timeout that
  // names no cause. Set a full PATH before anything looks up a binary.
  "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
  "export PATH",
  // Revert only what this script itself changed - a host that already had :80
  // open, or nginx already stopped, has to be left exactly as it was found.
  // The record lives on disk rather than under /run because a host rebooted
  // mid-renewal would otherwise forget it while the ufw rule it left behind
  // survives the reboot, and then nothing would ever close the port again.
  "STATE_DIR=/var/lib/nodenet-acme",
  "",
  "do_close() {",
  '  if [ -f "$STATE_DIR/stopped-nginx" ]; then',
  "    systemctl start nginx >/dev/null 2>&1",
  '    rm -f "$STATE_DIR/stopped-nginx"',
  "  fi",
  '  if [ -f "$STATE_DIR/opened-80" ]; then',
  "    ufw delete allow 80/tcp >/dev/null 2>&1",
  '    rm -f "$STATE_DIR/opened-80"',
  "  fi",
  "}",
  "",
  'case "$1" in',
  "  open)",
  '    mkdir -p "$STATE_DIR"',
  '    if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "Status: active"; then',
  // A rule scoped to a single source address still reads as "80 ALLOW" while
  // leaving the port shut to the CA, so only an allow from Anywhere counts.
  "      if ! ufw status | grep -qE '^80(/tcp)?( \\(v6\\))?[[:space:]]+ALLOW[[:space:]]+Anywhere'; then",
  // `ufw allow` appends, so a broader deny sitting earlier in the chain would
  // shadow it; insert at the top, falling back to append on an empty ruleset.
  "        if ufw insert 1 allow 80/tcp >/dev/null 2>&1 || ufw allow 80/tcp >/dev/null 2>&1; then",
  '          : > "$STATE_DIR/opened-80"',
  "        fi",
  "      fi",
  "    fi",
  "    if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then",
  '      systemctl stop nginx >/dev/null 2>&1 && : > "$STATE_DIR/stopped-nginx"',
  "    fi",
  "    ;;",
  "  close)",
  "    do_close",
  "    ;;",
  // A renewal takes seconds, so a flag still standing long afterwards belongs
  // to a run that was killed between its two hooks - by a signal, an OOM kill
  // or a power cut. Nothing else would ever restore that host, so cron calls
  // this to finish the job. The age check is what keeps it from tearing the
  // port out from under a renewal that is legitimately still running.
  "  reap)",
  '    if [ -n "$(find "$STATE_DIR" -maxdepth 1 -type f -mmin +15 2>/dev/null | head -n 1)" ]; then',
  "      do_close",
  "    fi",
  "    ;;",
  "esac",
  // The hook's own status must never decide the renewal's - a failed `ufw
  // delete` cannot be allowed to turn a successful renewal into a failure.
  "exit 0",
].join("\n");

const panelSslCommand = (host: string) => {
  const ip = shellQuote(host);
  const openHook = shellQuote(`${acmeHelperPath} open`);
  const closeHook = shellQuote(`${acmeHelperPath} close`);
  return [
    `cat > ${acmeHelperPath} <<'NODENET_ACME_HOOK'`,
    acmePortHelper,
    "NODENET_ACME_HOOK",
    `chmod 755 ${acmeHelperPath}`,
    // Belt and braces: should this script die between opening the port and the
    // ACME client's own post hook, the EXIT trap still restores the host.
    `trap ${closeHook} EXIT`,
    "trap 'exit 129' HUP",
    "trap 'exit 130' INT",
    "trap 'exit 143' TERM",
    // certbot runs every executable in these directories around each renewal it
    // performs, whichever certificate triggered it - so wiring it here covers
    // the certs on the box today and any issued later.
    "if [ -d /etc/letsencrypt ]; then",
    "  mkdir -p /etc/letsencrypt/renewal-hooks/pre /etc/letsencrypt/renewal-hooks/post",
    `  printf '#!/bin/sh\\nexec ${acmeHelperPath} open\\n' > /etc/letsencrypt/renewal-hooks/pre/nodenet-acme-port80`,
    `  printf '#!/bin/sh\\nexec ${acmeHelperPath} close\\n' > /etc/letsencrypt/renewal-hooks/post/nodenet-acme-port80`,
    "  chmod 755 /etc/letsencrypt/renewal-hooks/pre/nodenet-acme-port80 /etc/letsencrypt/renewal-hooks/post/nodenet-acme-port80",
    '  echo "certbot renewal hooks installed"',
    "fi",
    'ACME_HOME="$HOME/.acme.sh"',
    '[ -d "$ACME_HOME" ] || ACME_HOME=/root/.acme.sh',
    // 3x-ui keeps the panel's certificate path in its settings table and holds
    // that database open for writing - read it read-only, so an in-flight write
    // cannot make a configured panel look like it has no certificate at all.
    'PANEL_CERT=""',
    "if command -v sqlite3 >/dev/null 2>&1 && [ -f /etc/x-ui/x-ui.db ]; then",
    `  PANEL_CERT=$(sqlite3 -readonly -cmd '.timeout 5000' /etc/x-ui/x-ui.db "SELECT value FROM settings WHERE key = 'webCertFile' AND value != '';" 2>/dev/null) || PANEL_CERT=""`,
    "fi",
    'if [ -n "$PANEL_CERT" ] && [ -f "$PANEL_CERT" ]; then',
    '  echo "Panel already serves $PANEL_CERT - keeping it, only wiring renewals."',
    "else",
    `  echo "No panel certificate configured - issuing one for ${host}."`,
    // acme.sh's standalone challenge listener is built on socat.
    "  if ! command -v socat >/dev/null 2>&1; then",
    "    if command -v apt-get >/dev/null 2>&1; then apt-get update >/dev/null 2>&1 && apt-get install -y socat >/dev/null 2>&1;",
    "    elif command -v dnf >/dev/null 2>&1; then dnf install -y socat >/dev/null 2>&1;",
    "    elif command -v yum >/dev/null 2>&1; then yum install -y socat >/dev/null 2>&1;",
    "    elif command -v apk >/dev/null 2>&1; then apk add socat >/dev/null 2>&1; fi",
    "  fi",
    '  if [ ! -f "$ACME_HOME/acme.sh" ]; then',
    "    curl -s https://get.acme.sh | sh >/dev/null 2>&1",
    '    ACME_HOME="$HOME/.acme.sh"',
    "  fi",
    '  [ -f "$ACME_HOME/acme.sh" ] || { echo "acme.sh is missing and could not be installed" >&2; exit 1; }',
    '  "$ACME_HOME/acme.sh" --set-default-ca --server letsencrypt >/dev/null 2>&1',
    // Let's Encrypt issues for a bare IP only under its shortlived profile
    // (~6 days), which is exactly why renewal has to work unattended. Passing
    // the hooks here makes acme.sh save them into this certificate's own
    // renewal config, so every future cron run reuses them.
    `  "$ACME_HOME/acme.sh" --issue -d ${ip} --standalone --server letsencrypt --certificate-profile shortlived --days 6 --httpport 80 --pre-hook ${openHook} --post-hook ${closeHook} --force || { echo "Certificate issuance failed - see the output above." >&2; exit 1; }`,
    "  mkdir -p /root/cert/ip",
    // acme.sh exits non-zero when its reload command fails even though the cert
    // files were installed fine, so test for the files rather than the status.
    `  "$ACME_HOME/acme.sh" --installcert --force -d ${ip} --key-file /root/cert/ip/privkey.pem --fullchain-file /root/cert/ip/fullchain.pem --reloadcmd 'systemctl restart x-ui 2>/dev/null || rc-service x-ui restart 2>/dev/null' >/dev/null 2>&1 || true`,
    '  { [ -f /root/cert/ip/fullchain.pem ] && [ -f /root/cert/ip/privkey.pem ]; } || { echo "Certificate files were not installed under /root/cert/ip." >&2; exit 1; }',
    "  chmod 600 /root/cert/ip/privkey.pem",
    "  chmod 644 /root/cert/ip/fullchain.pem",
    "  if [ -x /usr/local/x-ui/x-ui ]; then",
    "    /usr/local/x-ui/x-ui cert -webCert /root/cert/ip/fullchain.pem -webCertKey /root/cert/ip/privkey.pem",
    "    systemctl restart x-ui >/dev/null 2>&1 || true",
    '    echo "Panel certificate set - the panel now answers over https."',
    "  else",
    '    echo "3x-ui binary not found at /usr/local/x-ui/x-ui - certificate issued but not attached to the panel." >&2',
    "  fi",
    "fi",
    // Certificates issued before this preset ran - by 3x-ui's own CLI, or by
    // hand - carry empty hooks in their saved renewal config, so their cron
    // renewals would still walk into the closed port. Point them all at the
    // helper. acme.sh stores hook commands base64-wrapped in these markers and
    // decodes them when it reloads the config to renew.
    `PRE_B64=$(printf '%s' ${openHook} | base64 | tr -d '\\n')`,
    `POST_B64=$(printf '%s' ${closeHook} | base64 | tr -d '\\n')`,
    "WIRED=0",
    'for conf in "$ACME_HOME"/*/*.conf; do',
    '  [ -f "$conf" ] || continue',
    `  grep -q '^Le_Domain=' "$conf" || continue`,
    `  pre_line=$(printf "Le_PreHook='__ACME_BASE64__START_%s__ACME_BASE64__END_'" "$PRE_B64")`,
    `  post_line=$(printf "Le_PostHook='__ACME_BASE64__START_%s__ACME_BASE64__END_'" "$POST_B64")`,
    `  if grep -q '^Le_PreHook=' "$conf"; then sed -i "s|^Le_PreHook=.*|$pre_line|" "$conf"; else printf '%s\\n' "$pre_line" >> "$conf"; fi`,
    `  if grep -q '^Le_PostHook=' "$conf"; then sed -i "s|^Le_PostHook=.*|$post_line|" "$conf"; else printf '%s\\n' "$post_line" >> "$conf"; fi`,
    "  WIRED=$((WIRED + 1))",
    "done",
    'echo "acme.sh renewal configs wired to the port helper: $WIRED"',
    // acme.sh renews only if its cron entry exists; a hand-installed acme.sh,
    // or one whose crontab was cleared, has none.
    `if [ -f "$ACME_HOME/acme.sh" ] && ! crontab -l 2>/dev/null | grep -q 'acme.sh --cron'; then`,
    '  "$ACME_HOME/acme.sh" --install-cronjob >/dev/null 2>&1 || true',
    "fi",
    `crontab -l 2>/dev/null | grep 'acme.sh --cron' || echo "warning: no acme.sh cron entry - renewals will not run unattended" >&2`,
    `if ! crontab -l 2>/dev/null | grep -q '${acmeHelperPath} reap'; then`,
    `  (crontab -l 2>/dev/null; echo '*/5 * * * * ${acmeHelperPath} reap >/dev/null 2>&1') | crontab -`,
    '  echo "port-80 reaper scheduled"',
    "fi",
    'echo "Done. Renewals now open port 80 and stop nginx by themselves, then restore both."',
  ].join("\n");
};

const presets: PresetItem[] = [
  {
    id: "sshKey",
    name: "Copy SSH public key",
    description: "Adds a selected ~/.ssh public key to authorized_keys on the server.",
    command: "",
    recommended: true,
  },
  {
    id: "install3xui",
    name: "Install 3x-ui panel",
    description: "Panel will be configured on port 65333.",
    command:
      "if command -v apt-get >/dev/null 2>&1; then apt-get update && apt-get install -y sqlite3; elif command -v dnf >/dev/null 2>&1; then dnf install -y sqlite; elif command -v yum >/dev/null 2>&1; then yum install -y sqlite; elif command -v apk >/dev/null 2>&1; then apk add sqlite; fi; printf 'y\\n65333\\n4\\nN\\n' | bash <(curl -Ls https://raw.githubusercontent.com/mhsanaei/3x-ui/master/install.sh)",
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
      "printf 'net.core.default_qdisc=fq\\nnet.ipv4.tcp_congestion_control=bbr\\n' > /etc/sysctl.d/99-nodenet-bbr.conf && sysctl --system",
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
    description: "Requires an authorized key, disables password auth, and restarts SSH.",
    command:
      "test -s ~/.ssh/authorized_keys || { echo 'No SSH public key found in ~/.ssh/authorized_keys. Run Copy SSH public key first.' >&2; exit 1; }; for file in /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf; do [ -f \"$file\" ] || continue; sed -i -E 's/^[#[:space:]]*PasswordAuthentication[[:space:]]+.*/PasswordAuthentication no/' \"$file\"; sed -i -E 's/^[#[:space:]]*PubkeyAuthentication[[:space:]]+.*/PubkeyAuthentication yes/' \"$file\"; sed -i -E 's/^[#[:space:]]*KbdInteractiveAuthentication[[:space:]]+.*/KbdInteractiveAuthentication no/' \"$file\"; done; printf 'PasswordAuthentication no\\nPubkeyAuthentication yes\\nKbdInteractiveAuthentication no\\n' > /etc/ssh/sshd_config.d/99-nodenet-hardening.conf && sshd -t && (systemctl restart ssh || systemctl restart sshd)",
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
    id: "panelSsl",
    name: "Panel SSL with auto-renewal",
    description:
      "Issues a Let's Encrypt certificate for the panel, serves the panel over HTTPS, and teaches renewals to open UFW and free port 80 on their own.",
    command: "",
    recommended: true,
    outputWindow: true,
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
    void invoke<string>("get_public_ip")
      .then((ip) => setManagementIp(ip))
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

  const runPreset = async (preset: PresetItem, rethrow = false, keepRunning = false) => {
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
      if (preset.id === "panelSsl") {
        await upgradePanelUrlToHttps();
      }
    } catch (err) {
      const error = err instanceof Error ? err : new Error(String(err));
      setError(error.message);
      if (rethrow) {
        throw error;
      }
    } finally {
      if (!keepRunning) setRunning(null);
    }
  };

  // The preset moves the panel onto TLS, so the stored panel URL has to follow
  // it: a saved http:// URL against an https listener fails every panel call -
  // inbounds, clients, traffic - with a connection error that reads like the
  // panel broke rather than like a stale scheme.
  const upgradePanelUrlToHttps = async () => {
    const url = server.panelUrl?.trim();
    if (!url || !url.toLowerCase().startsWith("http://")) return;
    const updated = { ...server, panelUrl: `https://${url.slice("http://".length)}` };
    if (onServerUpdated) {
      await onServerUpdated(updated);
    } else {
      await invoke("upsert_server", { server: updated });
    }
    setMessage("Panel URL switched to https");
  };

  const runSelected = async () => {
    setRunning("all");
    setError("");
    setMessage("");
    try {
      for (const preset of presets) {
        if (selected[preset.id]) {
          await runPreset(preset, true, true);
        }
      }
      if (selected.install3xui) {
        await fetchPanelSetupInfo();
      }
      setMessage("Selected setup presets finished");
      onDone?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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
      const sshPort = Math.max(1, Math.min(65535, Math.round(server.sshPort || 22)));
      return [
        "if ! command -v ufw >/dev/null 2>&1; then apt-get update && apt-get install -y ufw; fi",
        `ufw allow from ${ip} to any port ${sshPort}`,
        `ufw delete allow ${sshPort}/tcp || true`,
        sshPort === 22 ? "" : "ufw delete allow 22/tcp || true",
        "ufw allow 65333/tcp",
        "ufw allow 443/tcp",
        "yes | ufw enable",
        "ufw reload",
        "ufw status verbose",
      ].filter(Boolean).join(" && ");
    }

    if (preset.id === "panelSsl") {
      return panelSslCommand(server.host);
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
      const path = normalizePanelBasePath(info.webBasePath);
      setMessage(`3x-ui panel saved from ${info.source}: ${info.username} on port ${info.port}${path}`);
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

const normalizePanelBasePath = (value: string | null | undefined) => {
  const trimmed = (value ?? "").trim().replace(/^\/+|\/+$/g, "");
  return trimmed ? `/${trimmed}` : "";
};

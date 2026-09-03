import { useEffect, useMemo, useState } from "react";
import { KeyRound, RefreshCw } from "lucide-react";
import type { ServerConfig } from "../types";
import { useSslStore, type SslCertificateRow } from "../stores/sslStore";
import CommandOutputModal from "./CommandOutputModal";
import ConfirmModal from "./ConfirmModal";

interface SslCertificatesProps {
  servers: ServerConfig[];
}

const shellQuote = (value: string) => `'${value.replace(/'/g, "'\\''")}'`;

// Both the ufw rule and the nginx stop have to be reverted even when the
// shell dies mid-renewal: a dropped SSH session (SIGHUP) would otherwise
// leave port 80 open and, worse, nginx down. Revert from an EXIT trap so
// every path out of the script restores the host, and route the signals
// through `exit` so they land in that same handler.
const cleanupPreamble = [
  `UFW_ACTIVE=0; OPENED_PORT80=0; NGINX_STOPPED=0; CLEANED=0; RENEW_STATUS=1; PORT80_HOLDER=`,
  `cleanup() {`,
  `  [ "$CLEANED" = "1" ] && return`,
  `  CLEANED=1`,
  `  if [ "$NGINX_STOPPED" = "1" ]; then systemctl start nginx >/dev/null 2>&1; fi`,
  `  if [ "$OPENED_PORT80" = "1" ]; then ufw delete allow 80/tcp >/dev/null 2>&1 || true; fi`,
  `}`,
  `trap cleanup EXIT`,
  `trap 'exit 129' HUP`,
  `trap 'exit 130' INT`,
  `trap 'exit 143' TERM`,
];

const ufwOpenPreamble = [
  `if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then`,
  `  UFW_ACTIVE=1`,
  // Only an allow open to *any* source lets Let's Encrypt reach the
  // challenge; a rule scoped to one address ("80/tcp ALLOW 10.0.0.5") would
  // otherwise read as "already open" and leave the port shut to the CA.
  `  if ! ufw status | grep -qE '^80(/tcp)?( \\(v6\\))?[[:space:]]+ALLOW[[:space:]]+Anywhere'; then`,
  // `ufw allow` appends, so an existing broader deny sitting earlier in the
  // chain shadows it and the port never actually opens. Insert at the top
  // instead, falling back to append when the ruleset is empty (`insert 1`
  // is rejected when there is no rule 1 to insert before).
  `    if ufw insert 1 allow 80/tcp >/dev/null 2>&1 || ufw allow 80/tcp >/dev/null 2>&1; then`,
  `      OPENED_PORT80=1`,
  `    fi`,
  `  fi`,
  `fi`,
];

const captureRenewStatus = `RENEW_STATUS=$?`;

// A challenge the CA cannot reach fails with nothing but "Timeout during
// connect (likely firewall problem)", which never says *which* firewall.
// Dump the host's own view of port 80 while the rules are still in place, so
// a local block is distinguishable from one upstream of the machine.
const port80Diagnostics = [
  `if [ "$RENEW_STATUS" != "0" ]; then`,
  `  echo "--- port 80 diagnostics ---" >&2`,
  `  if [ -n "$PORT80_HOLDER" ]; then echo "$PORT80_HOLDER" >&2; else echo "nothing else was listening on :80 during the challenge" >&2; fi`,
  `  if [ "$UFW_ACTIVE" = "1" ]; then ufw status numbered >&2; fi`,
  `  command -v iptables >/dev/null 2>&1 && iptables -S INPUT 2>/dev/null | head -n 30 >&2`,
  `  command -v nft >/dev/null 2>&1 && nft list ruleset 2>/dev/null | head -n 30 >&2`,
  `  echo "If nothing above blocks :80, the block is upstream of this host - provider firewall / security group / ISP filtering inbound 80." >&2`,
  `fi`,
];

// Reverting is the EXIT trap's job; the script only has to name its status.
const exitPostamble = [`exit $RENEW_STATUS`];

// A squatter on :80 makes the standalone challenge unreachable just as a
// firewall does, and by the time the renewal has failed the port is free
// again — so snapshot the holder while the challenge is still live.
const capturePort80Holder = `PORT80_HOLDER="$(ss -tlnp 2>/dev/null | grep -E ':80[[:space:]]' || true)"`;

// Certbot has no record of an acme.sh-issued cert and vice versa (3x-ui's
// own CLI issues IP/domain certs via acme.sh straight to /root/cert/, not
// through certbot) — renewal must be dispatched to whichever client
// actually manages the cert, identified server-side by its file path.
const renewCommand = (cert: SslCertificateRow) => {
  if (cert.source === "acmeSh") {
    // acme.sh's own identifier is the domain/IP passed at issuance (`-d`),
    // which for 3x-ui's IP-cert flow is the real IP, not the "ip" folder
    // name — so use the SAN we parsed rather than certName.
    const domain = shellQuote(cert.domains[0] ?? cert.certName);
    // Unlike certbot, acme.sh has no "nginx plugin" — 3x-ui's CLI always
    // issues these in --standalone mode, which is baked into the cert's
    // saved renewal config and reused by plain `--renew`. Standalone binds
    // port 80 itself, so if nginx already holds it the renewal fails
    // outright ("tcp port 80 is already used ... Please stop it first").
    // Stop nginx for the renewal and restart it right after, same
    // track-and-revert pattern as the ufw rule above.
    const renewStep = [
      `if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then`,
      `  systemctl stop nginx >/dev/null 2>&1 && NGINX_STOPPED=1`,
      `fi`,
      capturePort80Holder,
      `~/.acme.sh/acme.sh --renew -d ${domain} --force`,
      captureRenewStatus,
    ];
    return [
      ...cleanupPreamble,
      ...ufwOpenPreamble,
      ...renewStep,
      ...port80Diagnostics,
      ...exitPostamble,
    ].join("\n");
  }

  const name = shellQuote(cert.certName);
  // certbot's nginx plugin ships as its own package (python3-certbot-nginx)
  // and is routinely missing on hosts where certbot itself is installed, so a
  // running nginx is not on its own evidence that `--nginx` will work: without
  // the plugin certbot refuses the whole renewal with "The requested nginx
  // plugin does not appear to be installed". Standalone is always available,
  // but it binds :80 itself, so nginx has to step aside for the challenge -
  // tracked and put back by the same cleanup trap that reverts the ufw rule.
  const renewStep = [
    `if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null && certbot plugins --non-interactive 2>/dev/null | grep -qE '^\\* nginx$'; then`,
    `  ${capturePort80Holder}`,
    `  certbot renew --cert-name ${name} --nginx --non-interactive --force-renewal`,
    `else`,
    `  if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then`,
    `    systemctl stop nginx >/dev/null 2>&1 && NGINX_STOPPED=1`,
    `  fi`,
    `  ${capturePort80Holder}`,
    `  certbot renew --cert-name ${name} --standalone --non-interactive --force-renewal`,
    `fi`,
  ];
  return [
    ...cleanupPreamble,
    ...ufwOpenPreamble,
    ...renewStep,
    captureRenewStatus,
    ...port80Diagnostics,
    ...exitPostamble,
  ].join("\n");
};

const installCertbotCommand = [
  `[ "$(id -u)" = "0" ] || SUDO=sudo`,
  `if command -v apt-get >/dev/null 2>&1; then $SUDO apt-get update && $SUDO apt-get install -y certbot python3-certbot-nginx`,
  `elif command -v dnf >/dev/null 2>&1; then $SUDO dnf install -y certbot python3-certbot-nginx`,
  `elif command -v yum >/dev/null 2>&1; then $SUDO yum install -y certbot python3-certbot-nginx`,
  `else echo "Unsupported package manager" >&2; exit 1`,
  `fi`,
].join("\n");

const formatDate = (value: string | null) =>
  value
    ? new Date(value).toLocaleDateString([], {
        day: "2-digit",
        month: "short",
        year: "numeric",
      })
    : "--";

const statusClass = (status: SslCertificateRow["status"]) => {
  if (status === "valid") return "status-label active";
  if (status === "expiring") return "status-label warn";
  if (status === "expired") return "status-label error";
  return "status-label";
};

const statusLabel = (
  status: SslCertificateRow["status"],
  expiresAt: string | null,
) => {
  if (status === "expired") return "expired";
  if (status === "expiring") {
    const days = expiresAt
      ? Math.max(
          0,
          Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 86_400_000),
        )
      : null;
    return days !== null ? `expires in ${days}d` : "expiring";
  }
  if (status === "valid") return "valid";
  return "unknown";
};

export default function SslCertificates({ servers }: SslCertificatesProps) {
  const {
    certificates,
    certbotInstalledByServer,
    errorByServer,
    isLoading,
    loadAllCertificates,
  } = useSslStore();
  const [pendingRenew, setPendingRenew] = useState<SslCertificateRow | null>(
    null,
  );
  const [streamingOutput, setStreamingOutput] = useState<{
    title: string;
    serverId: string;
    command: string;
  } | null>(null);

  const serverIds = servers.map((server) => server.id).join(",");
  useEffect(() => {
    if (servers.length > 0) void loadAllCertificates(servers);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverIds]);

  const serversWithoutCerts = useMemo(
    () =>
      servers.filter(
        (server) =>
          !certificates.some((cert) => cert.serverId === server.id) &&
          !errorByServer[server.id],
      ),
    [servers, certificates, errorByServer],
  );

  const refresh = () => void loadAllCertificates(servers);

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">SSL</p>
          <h2>Certificates</h2>
          <span className="server-target">
            {certificates.length} certificate(s) across {servers.length}{" "}
            server(s)
          </span>
        </div>
        <div className="header-actions">
          <button
            className="command-button"
            disabled={isLoading}
            onClick={refresh}
          >
            <RefreshCw size={16} className={isLoading ? "spin" : ""} />
            <span>Refresh</span>
          </button>
        </div>
      </header>

      {Object.entries(errorByServer).map(([serverId, message]) => (
        <div className="error-state" key={serverId}>
          <div>
            <strong>
              {servers.find((server) => server.id === serverId)?.name ??
                serverId}
            </strong>
            <span>{message}</span>
          </div>
        </div>
      ))}

      <section className="inbounds-panel">
        <div className="ssl-table header">
          <span>Server</span>
          <span>Domain(s)</span>
          <span>Issuer</span>
          <span>Issued</span>
          <span>Expires</span>
          <span>Status</span>
          <span>Action</span>
        </div>

        <div className="ssl-panel-scroll">
          {isLoading && certificates.length === 0
            ? Array.from({ length: 4 }, (_, index) => (
                <div key={index} className="ssl-table row skeleton-row">
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                  <span className="skeleton-line" />
                </div>
              ))
            : certificates.map((cert) => (
                <div
                  className="ssl-table row"
                  key={`${cert.serverId}:${cert.certName}`}
                >
                  <span>{cert.serverName}</span>
                  <span title={cert.domains.join(", ")}>
                    {cert.domains[0]}
                    {cert.domains.length > 1
                      ? ` +${cert.domains.length - 1}`
                      : ""}
                  </span>
                  <span>{cert.issuer || "--"}</span>
                  <span>{formatDate(cert.issuedAt)}</span>
                  <span>{formatDate(cert.expiresAt)}</span>
                  <span className={statusClass(cert.status)}>
                    {statusLabel(cert.status, cert.expiresAt)}
                  </span>
                  <span>
                    {cert.source === "unknown" ? (
                      <span
                        className="muted-note"
                        title="This certificate wasn't found under Certbot or acme.sh (3x-ui's CLI) — its origin is unknown, so automatic renewal isn't safe to offer. Renew it manually."
                      >
                        Manual renewal only
                      </span>
                    ) : (
                      <button
                        className="command-button"
                        onClick={() => setPendingRenew(cert)}
                      >
                        <RefreshCw size={14} />
                        <span>Renew</span>
                      </button>
                    )}
                  </span>
                </div>
              ))}

          {serversWithoutCerts.map((server) => (
            <div className="ssl-table row" key={server.id}>
              <span>{server.name}</span>
              <span>No certificates found</span>
              <span />
              <span />
              <span />
              <span />
              <span>
                {certbotInstalledByServer[server.id] === false ? (
                  <button
                    className="command-button"
                    onClick={() =>
                      setStreamingOutput({
                        title: `Install certbot on ${server.name}`,
                        serverId: server.id,
                        command: installCertbotCommand,
                      })
                    }
                  >
                    <KeyRound size={14} />
                    <span>Install certbot</span>
                  </button>
                ) : null}
              </span>
            </div>
          ))}

          {!isLoading && certificates.length === 0 && servers.length === 0 ? (
            <div className="empty-state table-empty">
              <span>Add a server to see its certificates</span>
            </div>
          ) : null}
        </div>
      </section>

      {pendingRenew ? (
        <ConfirmModal
          title={`Renew ${pendingRenew.certName}?`}
          message={
            pendingRenew.source === "acmeSh"
              ? `This forces a renewal on ${pendingRenew.serverName} via acme.sh (the client 3x-ui's own SSL menu uses), which counts against Let's Encrypt's weekly rate limit.`
              : `This forces a renewal on ${pendingRenew.serverName} via certbot, which counts against Let's Encrypt's weekly rate limit. If nginx is running, the --nginx plugin is used with no downtime; otherwise certbot falls back to --standalone.`
          }
          confirmLabel="Renew certificate"
          onCancel={() => setPendingRenew(null)}
          onConfirm={() => {
            setStreamingOutput({
              title: `Renew ${pendingRenew.certName}`,
              serverId: pendingRenew.serverId,
              command: renewCommand(pendingRenew),
            });
            setPendingRenew(null);
          }}
        />
      ) : null}

      {streamingOutput ? (
        <CommandOutputModal
          title={streamingOutput.title}
          serverId={streamingOutput.serverId}
          command={streamingOutput.command}
          onClose={() => setStreamingOutput(null)}
          onComplete={() => void loadAllCertificates(servers)}
        />
      ) : null}
    </main>
  );
}

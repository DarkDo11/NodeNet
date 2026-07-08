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

const ufwOpenPreamble = [
  `UFW_ACTIVE=0; OPENED_PORT80=0`,
  `if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then`,
  `  UFW_ACTIVE=1`,
  `  if ! ufw status | grep -qE '^80(/tcp)?[[:space:]]+ALLOW'; then`,
  `    ufw allow 80/tcp >/dev/null 2>&1 && OPENED_PORT80=1`,
  `  fi`,
  `fi`,
];

const ufwClosePostamble = [
  `RENEW_STATUS=$?`,
  `if [ "$OPENED_PORT80" = "1" ]; then`,
  `  ufw delete allow 80/tcp >/dev/null 2>&1 || true`,
  `fi`,
  `exit $RENEW_STATUS`,
];

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
      `NGINX_STOPPED=0`,
      `if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then`,
      `  systemctl stop nginx >/dev/null 2>&1 && NGINX_STOPPED=1`,
      `fi`,
      `~/.acme.sh/acme.sh --renew -d ${domain} --force`,
      `RENEW_STATUS=$?`,
      `if [ "$NGINX_STOPPED" = "1" ]; then`,
      `  systemctl start nginx >/dev/null 2>&1`,
      `fi`,
    ];
    return [...ufwOpenPreamble, ...renewStep, ...ufwClosePostamble.slice(1)].join("\n");
  }

  const name = shellQuote(cert.certName);
  const renewStep = [
    `if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then`,
    `  certbot renew --cert-name ${name} --nginx --non-interactive --force-renewal`,
    `else`,
    `  certbot renew --cert-name ${name} --standalone --non-interactive --force-renewal`,
    `fi`,
  ];
  return [...ufwOpenPreamble, ...renewStep, ...ufwClosePostamble].join("\n");
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
    ? new Date(value).toLocaleDateString([], { day: "2-digit", month: "short", year: "numeric" })
    : "--";

const statusClass = (status: SslCertificateRow["status"]) => {
  if (status === "valid") return "status-label active";
  if (status === "expiring") return "status-label warn";
  if (status === "expired") return "status-label error";
  return "status-label";
};

const statusLabel = (status: SslCertificateRow["status"], expiresAt: string | null) => {
  if (status === "expired") return "expired";
  if (status === "expiring") {
    const days = expiresAt
      ? Math.max(0, Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 86_400_000))
      : null;
    return days !== null ? `expires in ${days}d` : "expiring";
  }
  if (status === "valid") return "valid";
  return "unknown";
};

export default function SslCertificates({ servers }: SslCertificatesProps) {
  const { certificates, certbotInstalledByServer, errorByServer, isLoading, loadAllCertificates } =
    useSslStore();
  const [pendingRenew, setPendingRenew] = useState<SslCertificateRow | null>(null);
  const [streamingOutput, setStreamingOutput] = useState<
    { title: string; serverId: string; command: string } | null
  >(null);

  const serverIds = servers.map((server) => server.id).join(",");
  useEffect(() => {
    if (servers.length > 0) void loadAllCertificates(servers);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverIds]);

  const serversWithoutCerts = useMemo(
    () =>
      servers.filter(
        (server) =>
          !certificates.some((cert) => cert.serverId === server.id) && !errorByServer[server.id],
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
            {certificates.length} certificate(s) across {servers.length} server(s)
          </span>
        </div>
        <div className="header-actions">
          <button className="command-button" disabled={isLoading} onClick={refresh}>
            <RefreshCw size={16} className={isLoading ? "spin" : ""} />
            <span>Refresh</span>
          </button>
        </div>
      </header>

      {Object.entries(errorByServer).map(([serverId, message]) => (
        <div className="error-state" key={serverId}>
          <div>
            <strong>{servers.find((server) => server.id === serverId)?.name ?? serverId}</strong>
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
                <div className="ssl-table row" key={`${cert.serverId}:${cert.certName}`}>
                  <span>{cert.serverName}</span>
                  <span title={cert.domains.join(", ")}>
                    {cert.domains[0]}
                    {cert.domains.length > 1 ? ` +${cert.domains.length - 1}` : ""}
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
                      <button className="command-button" onClick={() => setPendingRenew(cert)}>
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

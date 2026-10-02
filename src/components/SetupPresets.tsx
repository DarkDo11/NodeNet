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
  | "panelSsl"
  | "adblockDns";

interface PresetItem {
  id: PresetId;
  name: string;
  description: string;
  command: string;
  recommended: boolean;
  outputWindow?: boolean;
}

// A panel certificate is only worth having if it renews itself, and acme.sh
// renews unattended out of its own crontab entry. An HTTP-01 challenge on a
// server set up the way these are has two obstacles: ufw keeps :80 shut, and
// nginx often holds the port. Stopping nginx to free it is not an option -
// nginx usually fronts the tunnels, so every renewal would cut every client
// off. Instead the certificate renews in webroot mode and the helper makes
// whatever already sits on :80 serve the token: nginx gets a temporary route
// for the panel's host and a graceful reload, and a free port gets a tiny
// socat responder. Nothing that is running gets stopped.
const acmeHelperPath = "/usr/local/sbin/nodenet-acme-port80";
const acmeStateDir = "/var/lib/nodenet-acme";
const acmeWebroot = `${acmeStateDir}/webroot`;

// SIGHUP makes 3x-ui rebuild its web server from the settings table, which is
// where the new certificate path lives. Since v3.0.2 that leaves xray running;
// older builds restart xray in-process, which is still no worse than the
// `systemctl restart` it replaces. The pattern pins the panel binary itself so
// neither xray (a child of it) nor an open `x-ui` menu script gets the signal.
const panelReloadCommand =
  "pkill -HUP -f '^/usr/local/x-ui/x-ui( |$)' >/dev/null 2>&1 || true";

const acmePortHelper = [
  "#!/bin/sh",
  "# Managed by NodeNet. Gets an ACME HTTP-01 challenge through :80 without stopping anything.",
  // cron runs hooks with a bare PATH (/usr/bin:/bin on Debian and Ubuntu) while
  // ufw lives in /usr/sbin, so an unqualified `ufw` is simply not found there:
  // the port never opens and the renewal dies with a challenge timeout that
  // names no cause. Set a full PATH before anything looks up a binary.
  "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
  "export PATH",
  // Revert only what this script itself changed - a host that already had :80
  // open has to be left exactly as it was found. The record lives on disk
  // rather than under /run because a host rebooted mid-renewal would otherwise
  // forget it while the ufw rule it left behind survives the reboot, and then
  // nothing would ever close the port again.
  `STATE_DIR=${acmeStateDir}`,
  `WEBROOT=${acmeWebroot}`,
  `SELF=${acmeHelperPath}`,
  "",
  "listeners() {",
  "  ss -Hltn 'sport = :80' 2>/dev/null",
  "}",
  "",
  "port80_owner() {",
  "  if command -v ss >/dev/null 2>&1; then",
  "    ss -Hltnp 'sport = :80' 2>/dev/null | head -n 1",
  "  elif systemctl is-active --quiet nginx 2>/dev/null; then",
  "    echo '\"nginx\"'",
  "  fi",
  "}",
  "",
  // The token is served by the helper itself, one socat child per request.
  // Only bare token names are looked up, so the path cannot walk out of the
  // challenge directory.
  "start_responder() {",
  "  if ! command -v socat >/dev/null 2>&1; then",
  "    echo 'nodenet-acme: socat is missing, nothing can answer the challenge on :80' >&2",
  "    return",
  "  fi",
  "  if [ -e /proc/net/if_inet6 ]; then addr=TCP6-LISTEN:80,ipv6only=0; else addr=TCP4-LISTEN:80; fi",
  '  socat -T 10 "$addr,reuseaddr,fork" EXEC:"$SELF serve" </dev/null >/dev/null 2>&1 &',
  '  echo $! > "$STATE_DIR/responder-pid"',
  "  sleep 1",
  '  kill -0 "$(cat "$STATE_DIR/responder-pid")" 2>/dev/null || echo "nodenet-acme: the challenge responder did not start on :80" >&2',
  "}",
  "",
  // The route is matched by Host, so it only ever sees requests for the
  // panel's own address and every other site on :80 keeps answering as before.
  // It joins nginx's existing :80 sockets rather than opening new ones - a
  // listen on an address nginx does not hold yet could fail to bind on reload.
  "add_nginx_route() {",
  '  name=$1',
  '  case "$name" in *:*) name="[$name]" ;; esac',
  '  socks=$(listeners)',
  '  listen=""',
  "  if printf '%s\\n' \"$socks\" | grep -qE '(^|[[:space:]])(0\\.0\\.0\\.0|\\*):80[[:space:]]'; then",
  '    listen="    listen 80;"',
  "  fi",
  "  if printf '%s\\n' \"$socks\" | grep -qE '(^|[[:space:]])\\[::\\]:80[[:space:]]'; then",
  '    listen="$listen',
  '    listen [::]:80;"',
  "  fi",
  '  [ -n "$listen" ] || listen="    listen 80;"',
  "  for dir in /etc/nginx/conf.d /etc/nginx/http.d /etc/nginx/sites-enabled; do",
  '    [ -d "$dir" ] || continue',
  '    conf="$dir/nodenet-acme.conf"',
  '    cat > "$conf" <<EOF',
  "# nodenet-acme-challenge: temporary, $SELF close removes it",
  "server {",
  "$listen",
  "    server_name $name;",
  "    location ^~ /.well-known/acme-challenge/ {",
  "        root $WEBROOT;",
  "        default_type text/plain;",
  "    }",
  "    location / {",
  "        return 404;",
  "    }",
  "}",
  "EOF",
  // A directory nginx.conf never includes would make the route silently dead,
  // so only a file that shows up in the full config dump counts.
  "    if nginx -t >/dev/null 2>&1 && nginx -T 2>/dev/null | grep -q 'nodenet-acme-challenge'; then",
  '      echo "$conf" > "$STATE_DIR/nginx-conf"',
  "      systemctl reload nginx >/dev/null 2>&1 || nginx -s reload >/dev/null 2>&1",
  "      sleep 1",
  "      return",
  "    fi",
  '    rm -f "$conf"',
  "  done",
  "  echo 'nodenet-acme: could not add the challenge route to nginx' >&2",
  "}",
  "",
  "do_close() {",
  '  if [ -f "$STATE_DIR/responder-pid" ]; then',
  '    pid=$(cat "$STATE_DIR/responder-pid")',
  // After a reboot the pid may belong to something else entirely.
  '    if [ -n "$pid" ] && grep -q socat "/proc/$pid/cmdline" 2>/dev/null; then',
  '      kill "$pid" 2>/dev/null',
  "    fi",
  '    rm -f "$STATE_DIR/responder-pid"',
  "  fi",
  '  if [ -f "$STATE_DIR/nginx-conf" ]; then',
  '    conf=$(cat "$STATE_DIR/nginx-conf")',
  '    case "$conf" in */nodenet-acme.conf) rm -f "$conf" ;; esac',
  "    systemctl reload nginx >/dev/null 2>&1 || nginx -s reload >/dev/null 2>&1",
  '    rm -f "$STATE_DIR/nginx-conf"',
  "  fi",
  // Left by an older helper that still stopped nginx, should a host be updated
  // in the middle of one of its renewals.
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
  '    mkdir -p "$STATE_DIR" "$WEBROOT/.well-known/acme-challenge"',
  '    chmod 755 "$STATE_DIR" "$WEBROOT" "$WEBROOT/.well-known" "$WEBROOT/.well-known/acme-challenge"',
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
  // No host means a hook an older NodeNet left on a standalone certificate:
  // acme.sh answers the challenge itself there, so the firewall is all that
  // is left to do.
  '    if [ -n "$2" ]; then',
  '      owner=$(port80_owner)',
  '      if [ -z "$owner" ]; then',
  "        start_responder",
  "      elif printf '%s' \"$owner\" | grep -q '\"nginx\"'; then",
  '        add_nginx_route "$2"',
  "      else",
  '        echo "nodenet-acme: :80 is held by something other than nginx and is left alone: $owner" >&2',
  "      fi",
  "    fi",
  "    ;;",
  "  close)",
  "    do_close",
  "    ;;",
  "  serve)",
  '    read -r method target version',
  '    file=""',
  '    case "$target" in',
  "      /.well-known/acme-challenge/*)",
  '        token=${target#/.well-known/acme-challenge/}',
  '        case "$token" in',
  "          ''|*[!A-Za-z0-9_-]*) ;;",
  '          *) file="$WEBROOT/.well-known/acme-challenge/$token" ;;',
  "        esac",
  "        ;;",
  "    esac",
  '    if [ -n "$file" ] && [ -f "$file" ]; then',
  "      printf 'HTTP/1.0 200 OK\\r\\nContent-Type: text/plain\\r\\nContent-Length: %s\\r\\nConnection: close\\r\\n\\r\\n' \"$(wc -c < \"$file\" | tr -d ' ')\"",
  '      cat "$file"',
  "    else",
  "      printf 'HTTP/1.0 404 Not Found\\r\\nContent-Length: 0\\r\\nConnection: close\\r\\n\\r\\n'",
  "    fi",
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

// The renewal's ufw rule opens all of :80, not just the challenge route, so for
// those seconds everything nginx serves there is reachable from anywhere. A
// redirect or a static stub is harmless; a proxy to an internal service or a
// status page is not, and it would have been hidden behind ufw until now. Only
// worth saying when ufw does keep :80 shut - otherwise nothing new is exposed.
// The scan reads http server blocks out of `nginx -T`: a block listening on :80
// (or on nothing, which means :80) that proxies, lists directories or reports
// status gets named, unless a server-level `return` answers every request
// before any location is reached.
const nginxPort80Audit = [
  "if command -v nginx >/dev/null 2>&1 && ss -Hltnp 'sport = :80' 2>/dev/null | grep -q '\"nginx\"' \\",
  "  && command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q 'Status: active' \\",
  "  && ! ufw status | grep -qE '^80(/tcp)?( \\(v6\\))?[[:space:]]+ALLOW[[:space:]]+Anywhere'; then",
  "  EXPOSED=$(nginx -T 2>/dev/null | awk '",
  "    function report() {",
  "      if (flags != \"\" && !redirect && (listen80 || !haslisten)) printf \"  %s:%s\\n\", (names == \"\" ? \" (no server_name)\" : names), flags",
  "    }",
  "    { sub(/#.*/, \"\") }",
  "    !instream && /(^|[ \\t;{])stream[ \\t]*\\{/ { instream = 1; sbase = depth }",
  "    !instream && !insrv && /(^|[ \\t;{])server[ \\t]*\\{/ { insrv = 1; base = depth; names = \"\"; flags = \"\"; redirect = 0; listen80 = 0; haslisten = 0 }",
  "    insrv && /(^|[ \\t;{])listen[ \\t]/ { haslisten = 1; if ($0 ~ /listen[ \\t]+([^ \\t;]*:)?80([ \\t;]|$)/) listen80 = 1 }",
  "    insrv && /^[ \\t]*server_name[ \\t]/ { s = $0; sub(/^[ \\t]*server_name[ \\t]+/, \"\", s); sub(/;.*/, \"\", s); names = names \" \" s }",
  "    insrv && depth == base + 1 && /^[ \\t]*return[ \\t]/ { redirect = 1 }",
  "    insrv && /(proxy|fastcgi|uwsgi|scgi|grpc)_pass[ \\t]|stub_status|autoindex[ \\t]+on|dav_methods/ { match($0, /(proxy|fastcgi|uwsgi|scgi|grpc)_pass[ \\t]+[^;]*|stub_status|autoindex[ \\t]+on|dav_methods[^;]*/); flags = flags \" [\" substr($0, RSTART, RLENGTH) \"]\" }",
  "    { o = gsub(/\\{/, \"{\"); c = gsub(/\\}/, \"}\"); depth += o - c }",
  "    insrv && depth <= base { report(); insrv = 0 }",
  "    instream && depth <= sbase { instream = 0 }",
  "  ')",
  '  if [ -n "$EXPOSED" ]; then',
  '    echo "warning: renewals open :80 to everyone for a few seconds, and nginx serves more than a redirect or a stub there:" >&2',
  "    printf '%s\\n' \"$EXPOSED\" >&2",
  '    echo "Restrict these in nginx itself (allow/deny, or listen on 127.0.0.1) - ufw only covers them between renewals." >&2',
  "  else",
  '    echo "nginx on :80 only redirects or serves static files - opening it during renewals exposes nothing new."',
  "  fi",
  "fi",
];

// The host goes into hook commands that acme.sh evals on every renewal, and
// into an nginx server_name, so anything beyond a hostname or IP is refused.
const isPlainHost = (host: string) => /^[A-Za-z0-9.:-]+$/.test(host);

const panelSslCommand = (host: string) => {
  if (!isPlainHost(host)) {
    return `echo ${shellQuote(`Refusing to issue a certificate for ${host}: not a plain hostname or IP.`)} >&2; exit 1`;
  }
  const ip = shellQuote(host);
  const openHook = shellQuote(`${acmeHelperPath} open ${host}`);
  const closeHook = shellQuote(`${acmeHelperPath} close`);
  const legacyOpenHook = shellQuote(`${acmeHelperPath} open`);
  const reloadCmd = shellQuote(panelReloadCommand);
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
    // Earlier versions hooked every certbot renewal on the box and stopped
    // nginx around each one. Only the panel's own certificate is NodeNet's
    // business, so those global hooks go.
    "for d in pre post; do",
    '  f="/etc/letsencrypt/renewal-hooks/$d/nodenet-acme-port80"',
    `  if [ -f "$f" ] && grep -q '${acmeHelperPath}' "$f"; then rm -f "$f"; echo "removed the old certbot $d hook"; fi`,
    "done",
    'ACME_HOME="$HOME/.acme.sh"',
    '[ -d "$ACME_HOME" ] || ACME_HOME=/root/.acme.sh',
    // 3x-ui keeps the panel's certificate path in its settings table and holds
    // that database open for writing - read it read-only, so an in-flight write
    // cannot make a configured panel look like it has no certificate at all.
    'PANEL_CERT=""',
    "if command -v sqlite3 >/dev/null 2>&1 && [ -f /etc/x-ui/x-ui.db ]; then",
    `  PANEL_CERT=$(sqlite3 -readonly -cmd '.timeout 5000' /etc/x-ui/x-ui.db "SELECT value FROM settings WHERE key = 'webCertFile' AND value != '';" 2>/dev/null) || PANEL_CERT=""`,
    "fi",
    // The challenge responder for a free :80 is built on socat.
    "if ! command -v socat >/dev/null 2>&1; then",
    "  if command -v apt-get >/dev/null 2>&1; then apt-get update >/dev/null 2>&1 && apt-get install -y socat >/dev/null 2>&1;",
    "  elif command -v dnf >/dev/null 2>&1; then dnf install -y socat >/dev/null 2>&1;",
    "  elif command -v yum >/dev/null 2>&1; then yum install -y socat >/dev/null 2>&1;",
    "  elif command -v apk >/dev/null 2>&1; then apk add socat >/dev/null 2>&1; fi",
    "fi",
    'if [ -n "$PANEL_CERT" ] && [ -f "$PANEL_CERT" ]; then',
    '  echo "Panel already serves $PANEL_CERT - keeping it, only wiring renewals."',
    "else",
    `  echo "No panel certificate configured - issuing one for ${host}."`,
    '  if [ ! -f "$ACME_HOME/acme.sh" ]; then',
    "    curl -s https://get.acme.sh | sh >/dev/null 2>&1",
    '    ACME_HOME="$HOME/.acme.sh"',
    "  fi",
    '  [ -f "$ACME_HOME/acme.sh" ] || { echo "acme.sh is missing and could not be installed" >&2; exit 1; }',
    '  "$ACME_HOME/acme.sh" --set-default-ca --server letsencrypt >/dev/null 2>&1',
    // Let's Encrypt issues for a bare IP only under its shortlived profile
    // (~6 days), which is exactly why renewal has to work unattended. Passing
    // the hooks here makes acme.sh save them, and webroot mode, into this
    // certificate's own renewal config, so every future cron run reuses them.
    `  "$ACME_HOME/acme.sh" --issue -d ${ip} --webroot ${acmeWebroot} --server letsencrypt --certificate-profile shortlived --days 6 --pre-hook ${openHook} --post-hook ${closeHook} --force || { echo "Certificate issuance failed - see the output above." >&2; exit 1; }`,
    "  mkdir -p /root/cert/ip",
    // acme.sh exits non-zero when its reload command fails even though the cert
    // files were installed fine, so test for the files rather than the status.
    `  "$ACME_HOME/acme.sh" --installcert --force -d ${ip} --key-file /root/cert/ip/privkey.pem --fullchain-file /root/cert/ip/fullchain.pem --reloadcmd ${reloadCmd} >/dev/null 2>&1 || true`,
    '  { [ -f /root/cert/ip/fullchain.pem ] && [ -f /root/cert/ip/privkey.pem ]; } || { echo "Certificate files were not installed under /root/cert/ip." >&2; exit 1; }',
    "  chmod 600 /root/cert/ip/privkey.pem",
    "  chmod 644 /root/cert/ip/fullchain.pem",
    "  if [ -x /usr/local/x-ui/x-ui ]; then",
    "    /usr/local/x-ui/x-ui cert -webCert /root/cert/ip/fullchain.pem -webCertKey /root/cert/ip/privkey.pem",
    `    ${panelReloadCommand}`,
    '    echo "Panel certificate set - the panel now answers over https."',
    "  else",
    '    echo "3x-ui binary not found at /usr/local/x-ui/x-ui - certificate issued but not attached to the panel." >&2',
    "  fi",
    "fi",
    // A panel certificate issued before this preset ran - by 3x-ui's own CLI,
    // or by hand - renews standalone with no hooks, which walks into the closed
    // port or into nginx. Switch exactly that certificate to webroot mode and
    // the helper. acme.sh stores hook commands base64-wrapped in these markers
    // and decodes them when it reloads the config to renew. Every other
    // certificate on the box belongs to its owner and is left as it is, except
    // for undoing the hooks an older NodeNet put on them.
    `PRE_B64=$(printf '%s' ${openHook} | base64 | tr -d '\\n')`,
    `POST_B64=$(printf '%s' ${closeHook} | base64 | tr -d '\\n')`,
    `RELOAD_B64=$(printf '%s' ${reloadCmd} | base64 | tr -d '\\n')`,
    `LEGACY_PRE_B64=$(printf '%s' ${legacyOpenHook} | base64 | tr -d '\\n')`,
    "WIRED=0",
    'for conf in "$ACME_HOME"/*/*.conf; do',
    '  [ -f "$conf" ] || continue',
    `  domain=$(sed -n "s/^Le_Domain=//p" "$conf" | tr -d "'")`,
    '  [ -n "$domain" ] || continue',
    `  if [ "$domain" = ${ip} ]; then`,
    `    for kv in "Le_Webroot='${acmeWebroot}'" "Le_PreHook='__ACME_BASE64__START_\${PRE_B64}__ACME_BASE64__END_'" "Le_PostHook='__ACME_BASE64__START_\${POST_B64}__ACME_BASE64__END_'" "Le_ReloadCmd='__ACME_BASE64__START_\${RELOAD_B64}__ACME_BASE64__END_'"; do`,
    '      key=${kv%%=*}',
    `      if grep -q "^$key=" "$conf"; then sed -i "s|^$key=.*|$kv|" "$conf"; else printf '%s\\n' "$kv" >> "$conf"; fi`,
    "    done",
    "    WIRED=$((WIRED + 1))",
    `  elif grep -q "^Le_PreHook=.*$LEGACY_PRE_B64" "$conf"; then`,
    `    sed -i '/^Le_PreHook=/d; /^Le_PostHook=/d' "$conf"`,
    '    echo "$domain: removed the hooks an older NodeNet added (they stopped nginx)"',
    `    if grep -q "^Le_Webroot='\\?no'\\?$" "$conf" && [ -n "$(ss -Hltn 'sport = :80' 2>/dev/null)" ]; then`,
    '      echo "warning: $domain renews standalone while :80 is taken - switch it to --webroot, --nginx or DNS-01, or its renewal will fail" >&2',
    "    fi",
    "  fi",
    "done",
    'echo "Panel certificate configs wired to the helper: $WIRED"',
    // A panel certificate from certbot is pointed at the same webroot and hooks
    // through its own renewal config. `reconfigure` dry-runs a renewal to prove
    // the new settings, which also proves the route through nginx works.
    'case "$PANEL_CERT" in',
    "  /etc/letsencrypt/live/*)",
    '    CERT_NAME=$(basename "$(dirname "$PANEL_CERT")")',
    `    if certbot reconfigure --cert-name "$CERT_NAME" --webroot -w ${acmeWebroot} --pre-hook ${openHook} --post-hook ${closeHook} --deploy-hook ${reloadCmd} >/dev/null 2>&1; then`,
    '      echo "certbot certificate $CERT_NAME switched to webroot renewals"',
    "    else",
    '      echo "warning: could not reconfigure certbot certificate $CERT_NAME (certbot 2.3+ needed) - its renewals are unchanged" >&2',
    "    fi",
    "    ;;",
    "esac",
    // acme.sh renews only if its cron entry exists; a hand-installed acme.sh,
    // or one whose crontab was cleared, has none.
    `if [ -f "$ACME_HOME/acme.sh" ] && ! crontab -l 2>/dev/null | grep -q 'acme.sh --cron'; then`,
    '  "$ACME_HOME/acme.sh" --install-cronjob >/dev/null 2>&1 || true',
    "fi",
    `if [ -f "$ACME_HOME/acme.sh" ]; then crontab -l 2>/dev/null | grep 'acme.sh --cron' || echo "warning: no acme.sh cron entry - renewals will not run unattended" >&2; fi`,
    `if ! crontab -l 2>/dev/null | grep -q '${acmeHelperPath} reap'; then`,
    `  (crontab -l 2>/dev/null; echo '*/5 * * * * ${acmeHelperPath} reap >/dev/null 2>&1') | crontab -`,
    '  echo "port-80 reaper scheduled"',
    "fi",
    ...nginxPort80Audit,
    'echo "Done. Renewals open port 80 and serve the challenge through nginx or a temporary responder - nothing gets stopped."',
  ].join("\n");
};

// Ad-blocking DNS for a VPN node. Xray's freedom outbound resolves through
// /etc/resolv.conf, so once unbound is the system resolver every ad and tracker
// domain on the list comes back NXDOMAIN and the connection is never opened.
// The list itself (~500k domains, built in the RavenVPN repo by
// tools/routing/build_ads.py) is too big to ship inside a command, so the helper
// downloads it and renders unbound's config on the server. Installed as a file
// so that re-running it - by hand or from here - is how the list gets updated,
// and `rollback` puts the old resolver back.
const adblockHelperPath = "/usr/local/sbin/nodenet-adblock-dns";

// String.raw keeps the awk escapes intact; the script deliberately avoids ${...}
// so nothing in it is taken for a template placeholder.
const adblockHelper = String.raw`#!/bin/bash
# Managed by NodeNet. Ad-blocking DNS for a VPN node: unbound becomes the system
# resolver, so Xray's freedom outbound (which resolves through /etc/resolv.conf)
# gets NXDOMAIN for ad and tracker domains and never opens the connection.
#   install  - install or update unbound and the blocklist
#   rollback - restore the previous resolver and stop unbound
set -e
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export PATH

BASE_URL=$NODENET_ADBLOCK_BASE_URL
[ -n "$BASE_URL" ] || BASE_URL=https://api.nox-net.site/static/happ-routing/ads-lists
D=/etc/unbound/unbound.conf.d
RPZ=/var/lib/unbound/ads.rpz
PREV=/root/nox-unbound.prev
# One hostname per line and nothing else: these lines are pasted into unbound's
# config verbatim, so a quote or a stray HTML page must never get through.
HOST_RE='^([a-z0-9_]([a-z0-9_-]{0,61}[a-z0-9-])?\.)+([a-z]{2,63}|xn--[a-z0-9-]{1,59})$'
IP_RE='^([0-9]{1,3}\.){3}[0-9]{1,3}(/[0-9]{1,2})?$'

die() { echo "error: $*" >&2; exit 1; }

rollback() {
  if [ -e /etc/resolv.conf.nox-bak ] || [ -L /etc/resolv.conf.nox-bak ]; then
    mv -f /etc/resolv.conf.nox-bak /etc/resolv.conf
  fi
  systemctl disable --now unbound 2>/dev/null || true
  rm -f "$D"/nox-*.conf "$RPZ"
  echo "Previous resolver restored, unbound stopped."
}

write_base_conf() {
  cat > "$1" <<'CONF'
# Managed by NodeNet (nodenet-adblock-dns). Local resolver for Xray.
server:
    interface: 127.0.0.1
    access-control: 127.0.0.0/8 allow
    num-threads: 1
    msg-cache-size: 32m
    rrset-cache-size: 64m
    prefetch: yes
    serve-expired: yes
    hide-identity: yes
    hide-version: yes
    module-config: "respip validator iterator"
    # A public name must never resolve to an internal address (DNS rebinding).
    private-address: 0.0.0.0/8
    private-address: 10.0.0.0/8
    private-address: 100.64.0.0/10
    private-address: 127.0.0.0/8
    private-address: 169.254.0.0/16
    private-address: 172.16.0.0/12
    private-address: 192.168.0.0/16
    private-address: ::/128
    private-address: ::1/128
    private-address: ::ffff:0:0/96
    private-address: fc00::/7
    private-address: fe80::/10
    tls-cert-bundle: /etc/ssl/certs/ca-certificates.crt

# Forward over DNS-over-TLS so the hosting provider cannot see the names.
forward-zone:
    name: "."
    forward-tls-upstream: yes
    forward-addr: 1.1.1.1@853#cloudflare-dns.com
    forward-addr: 8.8.8.8@853#dns.google
    forward-addr: 1.0.0.1@853#cloudflare-dns.com
CONF
}

install_all() {
  [ "$(id -u)" = 0 ] || die "run as root"
  command -v apt-get >/dev/null 2>&1 || die "only Debian/Ubuntu (apt) are supported"

  # Another resolver already bound to :53 would answer half the queries.
  other=$(ss -lnup 2>/dev/null | awk '$4 ~ /:53$/' | grep -oE '"[^"]+"' | tr -d '"' | sort -u | grep -vxE 'unbound|systemd-resolve' || true)
  [ -z "$other" ] || die "port 53 is already used by: $other - remove it first"

  if ! systemctl is-active --quiet unbound; then
    avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
    [ "$avail" -ge 600 ] || die "needs ~350 MB RAM, only $avail MB available"
  fi

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  echo "Downloading the blocklist from $BASE_URL"
  for f in ads-suffix.txt ads-exact.txt ads-allow.txt ads-manual-ips.txt; do
    curl -fsSL --max-time 180 "$BASE_URL/$f" -o "$tmp/$f" || die "download failed: $f"
  done
  for f in ads-suffix.txt ads-exact.txt ads-allow.txt; do
    if grep -vEq "$HOST_RE" "$tmp/$f"; then die "$f contains lines that are not hostnames"; fi
  done
  grep -vE '^[[:space:]]*(#|$)' "$tmp/ads-manual-ips.txt" | sed 's/#.*//; s/[[:space:]]//g' > "$tmp/ips"
  if grep -vEq "$IP_RE" "$tmp/ips"; then die "ads-manual-ips.txt contains lines that are not IPv4/CIDR"; fi
  n=$(wc -l < "$tmp/ads-suffix.txt")
  [ "$n" -ge 100000 ] || die "ads-suffix.txt looks truncated ($n lines)"

  { echo "server:"
    awk '{printf "    local-zone: \"%s.\" always_nxdomain\n", $1}' "$tmp/ads-suffix.txt"
    awk '{printf "    local-zone: \"%s.\" transparent\n", $1}' "$tmp/ads-allow.txt"
  } > "$tmp/nox-ads-zones.conf"
  { echo '$TTL 3600'
    echo '$ORIGIN ads.rpz.'
    echo "@ SOA localhost. root.localhost. $(date +%s) 86400 3600 604800 3600"
    echo "@ NS localhost."
    awk '{print $1 " CNAME ."}' "$tmp/ads-exact.txt"
    awk -F'[./]' '{p = (NF == 5) ? $5 : 32; print p "." $4 "." $3 "." $2 "." $1 ".rpz-ip CNAME ."}' "$tmp/ips"
  } > "$tmp/ads.rpz"
  write_base_conf "$tmp/nox-dns.conf"
  printf 'rpz:\n    name: ads.rpz.\n    zonefile: %s\n    rpz-log: no\n' "$RPZ" > "$tmp/nox-ads.conf"

  # The package's own helper must not touch resolv.conf; it is switched below,
  # and only once unbound has proved it answers correctly.
  [ -f /etc/default/unbound ] || printf 'RESOLVCONF=false\n' > /etc/default/unbound
  if ! command -v unbound >/dev/null 2>&1 || ! command -v dig >/dev/null 2>&1; then
    echo "Installing unbound"
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq unbound dnsutils ca-certificates >/dev/null
  fi

  rm -rf "$PREV" && mkdir -p "$PREV"
  cp -p "$D"/nox-*.conf "$RPZ" "$PREV"/ 2>/dev/null || true
  restore() {
    rm -f "$D"/nox-*.conf "$RPZ"
    cp -p "$PREV"/*.conf "$D"/ 2>/dev/null || true
    cp -p "$PREV"/ads.rpz "$RPZ" 2>/dev/null || true
    systemctl restart unbound 2>/dev/null || true
  }

  rm -f "$D"/nox-*.conf
  install -m 644 "$tmp/nox-dns.conf" "$D/nox-dns.conf"
  install -m 644 "$tmp/nox-ads.conf" "$D/nox-ads.conf"
  install -m 644 "$tmp/nox-ads-zones.conf" "$D/nox-ads-zones.conf"
  install -m 644 -o unbound "$tmp/ads.rpz" "$RPZ"
  unbound-checkconf >/dev/null || { restore; die "unbound rejected the config - previous one restored"; }
  systemctl enable -q unbound
  systemctl restart unbound

  st() { dig @127.0.0.1 +time=3 +tries=2 "$1" A | awk '/status:/{print $6}' | tr -d ,; }
  for _ in $(seq 20); do [ "$(st google.com)" = NOERROR ] && break; sleep 1; done
  ok=1
  [ "$(st google.com)" = NOERROR ] || ok=0
  [ "$(st doubleclick.net)" = NXDOMAIN ] || ok=0
  [ "$(dig @127.0.0.1 +time=5 +tries=2 +short 127.0.0.1.nip.io A | grep -cE '^[0-9.]+$')" = 0 ] || ok=0
  if [ "$ok" = 0 ]; then
    if grep -qx "nameserver 127.0.0.1" /etc/resolv.conf; then
      restore; die "unbound answers wrong - previous files restored"
    fi
    die "unbound answers wrong - resolv.conf left untouched"
  fi

  if ! grep -qx "nameserver 127.0.0.1" /etc/resolv.conf; then
    cp -P /etc/resolv.conf /etc/resolv.conf.nox-bak
    printf 'nameserver 127.0.0.1\noptions edns0 trust-ad\n' > /etc/resolv.conf.nox
    mv -f /etc/resolv.conf.nox /etc/resolv.conf
    echo "resolv.conf now points at unbound (previous one kept as /etc/resolv.conf.nox-bak)"
  fi

  # Xray only goes through unbound when it resolves via the system. A dns
  # section with its own servers in the 3x-ui template bypasses the blocklist.
  cfg=/usr/local/x-ui/bin/config.json
  if [ -f "$cfg" ] && command -v python3 >/dev/null 2>&1; then
    python3 - "$cfg" <<'PY' || true
import json, sys
c = json.load(open(sys.argv[1]))
servers = (c.get("dns") or {}).get("servers") or []
own = [s for s in servers if (s.get("address") if isinstance(s, dict) else s) not in ("localhost", "127.0.0.1")]
if own:
    print("warning: Xray has its own DNS servers %s - set dns.servers to [\"localhost\"] in the 3x-ui Xray template, or the blocklist is bypassed" % own)
PY
  fi

  echo "Done: $n domains blocked, unbound $(( $(ps -o rss= -C unbound | head -n 1) / 1024 )) MB, $(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo) MB RAM available."
}

ACTION=$1
[ -n "$ACTION" ] || ACTION=install
case "$ACTION" in
  install|update) install_all ;;
  rollback) rollback ;;
  *) die "usage: $0 install|rollback" ;;
esac`;

const adblockDnsCommand = () =>
  [
    `cat > ${adblockHelperPath} <<'NODENET_ADBLOCK_DNS'`,
    adblockHelper,
    "NODENET_ADBLOCK_DNS",
    `chmod 755 ${adblockHelperPath}`,
    `${adblockHelperPath} install`,
  ].join("\n");

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
      "Issues a Let's Encrypt certificate for the panel, serves the panel over HTTPS, and renews it through nginx or a temporary responder on :80 - without stopping nginx or the tunnels.",
    command: "",
    recommended: true,
    outputWindow: true,
  },
  {
    id: "adblockDns",
    name: "Ad-blocking DNS (unbound + RPZ)",
    description:
      "Makes unbound the server's resolver with ~500k ad and tracker domains blocked, DNS-over-TLS upstream and rebinding protection. Run again to update the list.",
    command: "",
    recommended: false,
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

    if (preset.id === "adblockDns") {
      return adblockDnsCommand();
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

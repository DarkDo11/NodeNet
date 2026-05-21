import { invoke } from "@tauri-apps/api/core";
import { ArrowUpCircle, RefreshCw, Search } from "lucide-react";
import { useRef, useState } from "react";
import CommandOutputModal from "./CommandOutputModal";
import CountryFlag from "./CountryFlag";
import type { ServerConfig } from "../types";

interface ServerVersions {
  xui: string;
  xray: string;
}

const VERSION_CHECK_CMD =
  `echo "xui:$(x-ui version 2>/dev/null | grep -oE '[0-9]+\\.[0-9]+\\.[0-9]+' | head -1 || echo 'n/a')"; echo "xray:$(xray version 2>/dev/null | grep -oE '[0-9]+\\.[0-9]+\\.[0-9]+' | head -1 || echo 'n/a')"`;

const UPDATE_CMD =
  `printf 'y\\n\\n\\n\\n\\n\\n' | bash <(curl -Ls https://raw.githubusercontent.com/mhsanaei/3x-ui/master/install.sh) && echo "--- Restarting x-ui ---" && (systemctl restart x-ui 2>/dev/null || x-ui restart 2>/dev/null || true) && echo "--- Done ---"`;

interface ServerUpdatesProps {
  servers: ServerConfig[];
}

export default function ServerUpdates({ servers }: ServerUpdatesProps) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [versions, setVersions] = useState<Record<string, ServerVersions | "loading" | "error">>({});
  const [checking, setChecking] = useState(false);
  const [updateQueue, setUpdateQueue] = useState<{ id: string; name: string }[]>([]);
  const [queueIdx, setQueueIdx] = useState(0);
  const allCheckboxRef = useRef<HTMLInputElement | null>(null);

  const allSelected = servers.length > 0 && servers.every((s) => selectedIds.has(s.id));
  const someSelected = servers.some((s) => selectedIds.has(s.id));

  if (allCheckboxRef.current) {
    allCheckboxRef.current.indeterminate = someSelected && !allSelected;
  }

  const toggleAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(servers.map((s) => s.id)));
  };

  const toggleServer = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const checkVersions = async () => {
    const ids = servers.filter((s) => selectedIds.has(s.id)).map((s) => s.id);
    if (ids.length === 0) return;
    setChecking(true);
    for (const id of ids) {
      setVersions((prev) => ({ ...prev, [id]: "loading" }));
      try {
        const output = await invoke<string>("run_preset_command", {
          serverId: id,
          command: VERSION_CHECK_CMD,
        });
        const xuiMatch = output.match(/xui:(.+)/);
        const xrayMatch = output.match(/xray:(.+)/);
        setVersions((prev) => ({
          ...prev,
          [id]: {
            xui: xuiMatch?.[1]?.trim() || "n/a",
            xray: xrayMatch?.[1]?.trim() || "n/a",
          },
        }));
      } catch {
        setVersions((prev) => ({ ...prev, [id]: "error" }));
      }
    }
    setChecking(false);
  };

  const startUpdate = () => {
    const queue = servers
      .filter((s) => selectedIds.has(s.id))
      .map((s) => ({ id: s.id, name: s.name }));
    if (queue.length === 0) return;
    setUpdateQueue(queue);
    setQueueIdx(0);
  };

  const advanceQueue = () => {
    if (queueIdx + 1 < updateQueue.length) {
      setQueueIdx((i) => i + 1);
    } else {
      setUpdateQueue([]);
      setQueueIdx(0);
    }
  };

  const current = updateQueue[queueIdx];

  return (
    <>
      <article className="settings-panel">
        <div className="settings-panel-header">
          <ArrowUpCircle size={18} />
          <h3>Updates</h3>
        </div>

        <div className="update-server-list">
          <label className="update-server-row update-server-row-all">
            <input
              type="checkbox"
              ref={allCheckboxRef}
              checked={allSelected}
              onChange={toggleAll}
            />
            <span>All servers</span>
          </label>
          {servers.map((server) => {
            const ver = versions[server.id];
            return (
              <label key={server.id} className="update-server-row">
                <input
                  type="checkbox"
                  checked={selectedIds.has(server.id)}
                  onChange={() => toggleServer(server.id)}
                />
                <CountryFlag country={server.country} />
                <span className="update-server-name">{server.name}</span>
                {ver === "loading" ? (
                  <RefreshCw size={11} className="spin update-version-loading" />
                ) : ver === "error" ? (
                  <span className="update-version update-version-error">Error</span>
                ) : ver ? (
                  <span className="update-version">
                    {ver.xui} · {ver.xray}
                  </span>
                ) : null}
              </label>
            );
          })}
          {servers.length === 0 && (
            <p className="settings-hint">No servers configured.</p>
          )}
        </div>

        <div className="settings-actions">
          <button
            className="command-button"
            disabled={!someSelected || checking}
            onClick={() => void checkVersions()}
          >
            <Search size={16} className={checking ? "spin" : ""} />
            <span>{checking ? "Checking" : "Check versions"}</span>
          </button>
          <button
            className="command-button primary"
            disabled={!someSelected || updateQueue.length > 0}
            onClick={startUpdate}
          >
            <ArrowUpCircle size={16} />
            <span>Update & restart</span>
          </button>
        </div>
      </article>

      {current ? (
        <CommandOutputModal
          title={`Update ${current.name}${updateQueue.length > 1 ? ` (${queueIdx + 1}/${updateQueue.length})` : ""}`}
          serverId={current.id}
          command={UPDATE_CMD}
          onClose={advanceQueue}
        />
      ) : null}
    </>
  );
}

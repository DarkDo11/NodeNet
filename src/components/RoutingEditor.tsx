import {
  ArrowDown,
  ArrowUp,
  Pencil,
  Plus,
  RefreshCw,
  Route,
  Save,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { JsonObject, JsonValue, ServerConfig, XrayConfig } from "../types";

interface RoutingEditorProps {
  server: ServerConfig | null;
  config: XrayConfig | null;
  error?: string;
  isLoading: boolean;
  isSaving: boolean;
  onRefresh: () => void;
  onSave: (config: XrayConfig) => Promise<void>;
}

type RuleKind = "field" | "domain" | "ip" | "port";
type OutboundProtocol = "freedom" | "blackhole" | "socks" | "http";

interface RuleForm {
  kind: RuleKind;
  value: string;
  outboundTag: string;
}

interface OutboundForm {
  tag: string;
  protocol: OutboundProtocol;
  address: string;
  port: string;
}

const emptyRuleForm: RuleForm = {
  kind: "domain",
  value: "",
  outboundTag: "direct",
};

const emptyOutboundForm: OutboundForm = {
  tag: "",
  protocol: "socks",
  address: "",
  port: "",
};

const builtInOutboundTags = new Set(["direct", "block", "blocked", "default"]);

export default function RoutingEditor({
  server,
  config,
  error,
  isLoading,
  isSaving,
  onRefresh,
  onSave,
}: RoutingEditorProps) {
  const [draft, setDraft] = useState<XrayConfig | null>(null);
  const [dirty, setDirty] = useState(false);
  const [ruleFormOpen, setRuleFormOpen] = useState(false);
  const [ruleForm, setRuleForm] = useState<RuleForm>(emptyRuleForm);
  const [outboundFormOpen, setOutboundFormOpen] = useState(false);
  const [outboundForm, setOutboundForm] = useState<OutboundForm>(emptyOutboundForm);
  const [editingOutboundIndex, setEditingOutboundIndex] = useState<number | null>(null);

  useEffect(() => {
    if (config) {
      setDraft(cloneConfig(config));
      setDirty(false);
    } else {
      setDraft(null);
    }
  }, [config]);

  const ruleEntries = useMemo(() => getRuleEntries(draft), [draft]);
  const outboundEntries = useMemo(
    () => getOutboundEntries(draft).filter(({ outbound }) => !isBuiltInOutbound(outbound)),
    [draft],
  );
  const outboundTags = useMemo(() => {
    const tags = getOutboundEntries(draft)
      .map(({ outbound }) => stringField(outbound, "tag"))
      .filter((tag): tag is string => Boolean(tag));
    return tags.length > 0 ? tags : ["direct", "block"];
  }, [draft]);

  const updateDraft = (mutator: (next: XrayConfig) => void) => {
    setDraft((current) => {
      const next = cloneConfig(current ?? {});
      mutator(next);
      return next;
    });
    setDirty(true);
  };

  const addRule = () => {
    if (!ruleForm.value.trim() || !ruleForm.outboundTag.trim()) return;
    updateDraft((next) => {
      ensureRules(next).push(buildRule(ruleForm));
    });
    setRuleForm(emptyRuleForm);
    setRuleFormOpen(false);
  };

  const deleteRule = (index: number) => {
    updateDraft((next) => {
      ensureRules(next).splice(index, 1);
    });
  };

  const moveRule = (index: number, direction: -1 | 1) => {
    updateDraft((next) => {
      const rules = ensureRules(next);
      const target = index + direction;
      if (target < 0 || target >= rules.length) return;
      [rules[index], rules[target]] = [rules[target], rules[index]];
    });
  };

  const submitOutbound = () => {
    if (!canSubmitOutbound(outboundForm)) return;
    updateDraft((next) => {
      const outbounds = ensureOutbounds(next);
      const outbound = buildOutbound(outboundForm);
      if (editingOutboundIndex === null) {
        outbounds.push(outbound);
      } else {
        const current = asObject(outbounds[editingOutboundIndex]);
        outbounds[editingOutboundIndex] = current ? { ...current, ...outbound } : outbound;
      }
    });
    setOutboundForm(emptyOutboundForm);
    setOutboundFormOpen(false);
    setEditingOutboundIndex(null);
  };

  const startEditOutbound = (outbound: JsonObject, index: number) => {
    setOutboundForm(outboundToForm(outbound));
    setOutboundFormOpen(true);
    setEditingOutboundIndex(index);
  };

  const deleteOutbound = (index: number) => {
    updateDraft((next) => {
      ensureOutbounds(next).splice(index, 1);
    });
  };

  const save = async () => {
    if (!draft) return;
    await onSave(draft);
    setDirty(false);
  };

  if (!server) {
    return (
      <main className="content">
        <div className="empty-state">
          <Route size={28} />
          <h2>No server selected</h2>
        </div>
      </main>
    );
  }

  return (
    <main className="content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">3x-ui</p>
          <h2>Routing</h2>
          <span className="server-target">{server.panelUrl ?? "panelUrl is not configured"}</span>
        </div>
        <div className="header-actions">
          <button className="command-button" disabled={!server.panelUrl || isLoading} onClick={onRefresh}>
            <RefreshCw size={16} className={isLoading ? "spin" : ""} />
            <span>Refresh</span>
          </button>
          <button className="command-button primary" disabled={!draft || !dirty || isSaving} onClick={() => void save()}>
            <Save size={16} />
            <span>{isSaving ? "Saving" : "Save"}</span>
          </button>
        </div>
      </header>

      {!server.panelUrl ? (
        <div className="error-state">
          <div>
            <strong>Panel unavailable</strong>
            <span>Configure a 3x-ui panel URL for this server first.</span>
          </div>
        </div>
      ) : null}

      {error ? (
        <div className="error-state">
          <div>
            <strong>Xray config unavailable</strong>
            <span>{error}</span>
          </div>
          <button className="command-button" onClick={onRefresh}>Retry</button>
        </div>
      ) : null}

      <section className="routing-panel">
        <div className="routing-section-header">
          <div>
            <h3>Routing rules</h3>
            <span>{ruleEntries.length} rules</span>
          </div>
          <button className="command-button" disabled={!draft} onClick={() => setRuleFormOpen(true)}>
            <Plus size={16} />
            <span>Add rule</span>
          </button>
        </div>

        {ruleFormOpen ? (
          <div className="routing-inline-form rule-form">
            <label className="field">
              <span>Type</span>
              <select
                value={ruleForm.kind}
                onChange={(event) =>
                  setRuleForm((current) => ({ ...current, kind: event.target.value as RuleKind }))
                }
              >
                <option value="domain">Domain</option>
                <option value="ip">IP</option>
                <option value="port">Port</option>
                <option value="field">Field</option>
              </select>
            </label>
            <label className="field">
              <span>Value</span>
              <input value={ruleForm.value} onChange={(event) => setRuleForm((current) => ({ ...current, value: event.target.value }))} />
            </label>
            <label className="field">
              <span>Outbound tag</span>
              <select
                value={ruleForm.outboundTag}
                onChange={(event) =>
                  setRuleForm((current) => ({ ...current, outboundTag: event.target.value }))
                }
              >
                {outboundTags.map((tag) => (
                  <option value={tag} key={tag}>{tag}</option>
                ))}
              </select>
            </label>
            <div className="routing-form-actions">
              <button className="icon-button" onClick={() => setRuleFormOpen(false)} title="Cancel">
                <X size={16} />
              </button>
              <button className="command-button primary" disabled={!ruleForm.value.trim()} onClick={addRule}>
                <Plus size={16} />
                <span>Add</span>
              </button>
            </div>
          </div>
        ) : null}

        <div className="routing-table rules header">
          <span>Type</span>
          <span>Domain/IP/Port</span>
          <span>Outbound tag</span>
          <span>Actions</span>
        </div>
        {ruleEntries.map(({ rule, index }, visibleIndex) => (
          <div className="routing-table rules row" key={`${index}:${formatRuleTarget(rule)}`}>
            <span className="status-label info">{ruleKind(rule)}</span>
            <code>{formatRuleTarget(rule)}</code>
            <span>{stringField(rule, "outboundTag") ?? "direct"}</span>
            <span className="row-actions">
              <button className="icon-button" disabled={visibleIndex === 0} onClick={() => moveRule(index, -1)} title="Move up">
                <ArrowUp size={15} />
              </button>
              <button className="icon-button" disabled={visibleIndex === ruleEntries.length - 1} onClick={() => moveRule(index, 1)} title="Move down">
                <ArrowDown size={15} />
              </button>
              <button className="icon-button danger" onClick={() => deleteRule(index)} title="Delete rule">
                <Trash2 size={15} />
              </button>
            </span>
          </div>
        ))}
        {!isLoading && ruleEntries.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No routing rules in this config</span>
          </div>
        ) : null}
      </section>

      <section className="routing-panel">
        <div className="routing-section-header">
          <div>
            <h3>Outbound proxies</h3>
            <span>{outboundEntries.length} editable outbounds</span>
          </div>
          <button className="command-button" disabled={!draft} onClick={() => {
            setOutboundForm(emptyOutboundForm);
            setEditingOutboundIndex(null);
            setOutboundFormOpen(true);
          }}>
            <Plus size={16} />
            <span>Add outbound</span>
          </button>
        </div>

        {outboundFormOpen ? (
          <div className="routing-inline-form outbound-form">
            <label className="field">
              <span>Tag</span>
              <input value={outboundForm.tag} onChange={(event) => setOutboundForm((current) => ({ ...current, tag: event.target.value }))} />
            </label>
            <label className="field">
              <span>Protocol</span>
              <select
                value={outboundForm.protocol}
                onChange={(event) =>
                  setOutboundForm((current) => ({ ...current, protocol: event.target.value as OutboundProtocol }))
                }
              >
                <option value="freedom">Freedom</option>
                <option value="blackhole">Blackhole</option>
                <option value="socks">SOCKS</option>
                <option value="http">HTTP</option>
              </select>
            </label>
            <label className="field">
              <span>Server address</span>
              <input value={outboundForm.address} onChange={(event) => setOutboundForm((current) => ({ ...current, address: event.target.value }))} />
            </label>
            <label className="field">
              <span>Port</span>
              <input type="number" min={0} value={outboundForm.port} onChange={(event) => setOutboundForm((current) => ({ ...current, port: event.target.value }))} />
            </label>
            <div className="routing-form-actions">
              <button className="icon-button" onClick={() => {
                setOutboundFormOpen(false);
                setEditingOutboundIndex(null);
              }} title="Cancel">
                <X size={16} />
              </button>
              <button className="command-button primary" disabled={!canSubmitOutbound(outboundForm)} onClick={submitOutbound}>
                <Save size={16} />
                <span>{editingOutboundIndex === null ? "Add" : "Update"}</span>
              </button>
            </div>
          </div>
        ) : null}

        <div className="routing-table outbounds header">
          <span>Tag</span>
          <span>Protocol</span>
          <span>Address:Port</span>
          <span>Actions</span>
        </div>
        {outboundEntries.map(({ outbound, index }) => (
          <div className="routing-table outbounds row" key={`${index}:${stringField(outbound, "tag") ?? "outbound"}`}>
            <strong>{stringField(outbound, "tag") ?? "untagged"}</strong>
            <span className="status-label active">{stringField(outbound, "protocol") ?? "unknown"}</span>
            <code>{outboundAddress(outbound) || "-"}</code>
            <span className="row-actions">
              <button className="icon-button" onClick={() => startEditOutbound(outbound, index)} title="Edit outbound">
                <Pencil size={15} />
              </button>
              <button className="icon-button danger" onClick={() => deleteOutbound(index)} title="Delete outbound">
                <Trash2 size={15} />
              </button>
            </span>
          </div>
        ))}
        {!isLoading && outboundEntries.length === 0 ? (
          <div className="empty-state table-empty">
            <span>No editable outbound proxies</span>
          </div>
        ) : null}
      </section>
    </main>
  );
}

const cloneConfig = (value: XrayConfig | JsonObject): XrayConfig =>
  JSON.parse(JSON.stringify(value)) as XrayConfig;

const isObject = (value: JsonValue | undefined): value is JsonObject =>
  Boolean(value) && typeof value === "object" && !Array.isArray(value);

const asObject = (value: JsonValue | undefined): JsonObject | null =>
  isObject(value) ? value : null;

const stringField = (value: JsonObject, key: string) => {
  const item = value[key];
  return typeof item === "string" || typeof item === "number" ? String(item) : null;
};

const ensureObject = (config: XrayConfig, key: string) => {
  if (!isObject(config[key])) {
    config[key] = {};
  }
  return config[key] as JsonObject;
};

const ensureRules = (config: XrayConfig) => {
  const routing = ensureObject(config, "routing");
  if (!Array.isArray(routing.rules)) {
    routing.rules = [];
  }
  return routing.rules as JsonValue[];
};

const ensureOutbounds = (config: XrayConfig) => {
  if (!Array.isArray(config.outbounds)) {
    config.outbounds = [];
  }
  return config.outbounds as JsonValue[];
};

const getRuleEntries = (config: XrayConfig | null) => {
  const routing = config ? asObject(config.routing) : null;
  const rules = Array.isArray(routing?.rules) ? routing.rules : [];
  return rules
    .map((rule, index) => (isObject(rule) ? { rule, index } : null))
    .filter((entry): entry is { rule: JsonObject; index: number } => entry !== null);
};

const getOutboundEntries = (config: XrayConfig | null) => {
  const outbounds = Array.isArray(config?.outbounds) ? config.outbounds : [];
  return outbounds
    .map((outbound, index) => (isObject(outbound) ? { outbound, index } : null))
    .filter((entry): entry is { outbound: JsonObject; index: number } => entry !== null);
};

const splitValues = (value: string) =>
  value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);

const buildRule = (form: RuleForm): JsonObject => {
  const rule: JsonObject = {
    type: "field",
    outboundTag: form.outboundTag.trim(),
  };
  if (form.kind === "port") {
    rule.port = form.value.trim();
  } else if (form.kind === "ip") {
    rule.ip = splitValues(form.value);
  } else {
    rule.domain = splitValues(form.value);
  }
  return rule;
};

const ruleKind = (rule: JsonObject) => {
  if (Array.isArray(rule.domain)) return "domain";
  if (Array.isArray(rule.ip)) return "ip";
  if (rule.port !== undefined) return "port";
  return stringField(rule, "type") ?? "field";
};

const formatRuleTarget = (rule: JsonObject) => {
  if (Array.isArray(rule.domain)) return rule.domain.map(String).join(", ");
  if (Array.isArray(rule.ip)) return rule.ip.map(String).join(", ");
  if (rule.port !== undefined) return String(rule.port);
  return "-";
};

const isBuiltInOutbound = (outbound: JsonObject) => {
  const tag = stringField(outbound, "tag")?.toLowerCase();
  return tag ? builtInOutboundTags.has(tag) : false;
};

const firstServer = (outbound: JsonObject) => {
  const settings = asObject(outbound.settings);
  const servers = Array.isArray(settings?.servers) ? settings.servers : [];
  const server = servers.find((item): item is JsonObject => isObject(item));
  return server ?? null;
};

const outboundAddress = (outbound: JsonObject) => {
  const server = firstServer(outbound);
  if (!server) return "";
  const address = stringField(server, "address") ?? "";
  const port = stringField(server, "port") ?? "";
  return address && port ? `${address}:${port}` : address || port;
};

const outboundToForm = (outbound: JsonObject): OutboundForm => {
  const server = firstServer(outbound);
  const protocol = stringField(outbound, "protocol");
  return {
    tag: stringField(outbound, "tag") ?? "",
    protocol: protocol === "freedom" || protocol === "blackhole" || protocol === "http" ? protocol : "socks",
    address: server ? stringField(server, "address") ?? "" : "",
    port: server ? stringField(server, "port") ?? "" : "",
  };
};

const canSubmitOutbound = (form: OutboundForm) => {
  if (!form.tag.trim()) return false;
  if (form.protocol === "freedom" || form.protocol === "blackhole") return true;
  return Boolean(form.address.trim() && Number(form.port) > 0);
};

const buildOutbound = (form: OutboundForm): JsonObject => {
  const outbound: JsonObject = {
    tag: form.tag.trim(),
    protocol: form.protocol,
  };
  if (form.protocol === "socks" || form.protocol === "http") {
    outbound.settings = {
      servers: [
        {
          address: form.address.trim(),
          port: Number(form.port),
        },
      ],
    };
  } else if (form.protocol === "blackhole") {
    outbound.settings = { response: { type: "http" } };
  } else {
    outbound.settings = {};
  }
  return outbound;
};

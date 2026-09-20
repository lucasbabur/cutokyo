import {
  Boxes,
  ChevronRight,
  Network,
  Puzzle,
  ShieldCheck,
} from "lucide-react";
import { useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  InventoryKind,
  InventoryItem,
  PluginVerification,
} from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  EmptyState,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  ProvenanceDetails,
  RouteNotice,
  StatusPill,
} from "../components/Primitives.js";

const TABS: readonly {
  readonly id: "all" | InventoryKind;
  readonly label: string;
}[] = [
  { id: "all", label: "All" },
  { id: "mcp", label: "MCP servers" },
  { id: "plugin", label: "Plugins" },
  { id: "hook", label: "Hooks" },
  { id: "skill", label: "Skills" },
];

const HARNESS_NAMES = {
  claude_code: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
} as const;

export function InventoryPage() {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getInventory(),
    "inventory",
  );
  const [tab, setTab] = useState<"all" | InventoryKind>("all");
  const [busyItem, setBusyItem] = useState<string | null>(null);
  const [verification, setVerification] = useState<
    PluginVerification | "loading" | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const announce = useAnnounce();

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Discovering installed agent infrastructure" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Inventory is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data;
  if (data === null) return null;
  const visible =
    tab === "all" ? data.items : data.items.filter((item) => item.kind === tab);

  const toggleMcp = async (item: InventoryItem) => {
    setBusyItem(item.id);
    setError(null);
    try {
      const enabled = item.state !== "enabled";
      await commands.setMcpEnabled(item.id, enabled);
      announce(
        `${item.name} ${enabled ? "enabled" : "disabled"} for ${item.harnesses.map((harness) => HARNESS_NAMES[harness]).join(", ")}.`,
      );
      resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusyItem(null);
    }
  };

  const openVerification = async (itemId: string) => {
    setVerification("loading");
    setError(null);
    try {
      setVerification(await commands.getPluginVerification(itemId));
    } catch (reason) {
      setVerification(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  return (
    <div className="page">
      <PageHeader
        eyebrow="Scope · origin · effective state"
        title="Agent inventory"
        description="One attributable view of installed hooks, skills, plugins, and MCP servers. Cutokyo-owned entries stay distinct from user-owned configuration."
        actions={
          <StatusPill state={data.brokerState}>
            Central broker {data.brokerState}
          </StatusPill>
        }
      />
      <RouteNotice meta={data.meta} />
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <section className="broker-summary" aria-labelledby="broker-heading">
        <Network aria-hidden="true" />
        <div>
          <h2 id="broker-heading">Central MCP control</h2>
          <p>
            Approved upstream servers are namespaced and synchronized across
            harnesses. A failed upstream is contained; Cutokyo search remains a
            separate read-only MCP.
          </p>
        </div>
        <StatusPill state={data.searchMcpEnabled ? "enabled" : "disabled"}>
          Search MCP {data.searchMcpEnabled ? "enabled" : "disabled"}
        </StatusPill>
      </section>

      <div className="tabs" role="tablist" aria-label="Inventory type">
        {TABS.map((item) => (
          <button
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            className={tab === item.id ? "tab is-active" : "tab"}
            onClick={() => setTab(item.id)}
            key={item.id}
          >
            {item.label}
            <span>
              {item.id === "all"
                ? data.items.length
                : data.items.filter((entry) => entry.kind === item.id).length}
            </span>
          </button>
        ))}
      </div>

      {visible.length === 0 ? (
        <EmptyState
          compact
          title={`No ${tab === "all" ? "inventory items" : `${tab} items`} discovered`}
          description="Discovery completed for the current harness coverage. Unavailable config surfaces remain labelled in setup coverage rather than counted as empty."
        />
      ) : (
        <div className="inventory-list">
          {visible.map((item) => (
            <article className="inventory-row" key={item.id}>
              <div
                className={`inventory-row__icon inventory-row__icon--${item.kind}`}
                aria-hidden="true"
              >
                {item.kind === "mcp" ? (
                  <Network />
                ) : item.kind === "plugin" ? (
                  <Puzzle />
                ) : (
                  <Boxes />
                )}
              </div>
              <div className="inventory-row__body">
                <div className="inventory-row__title">
                  <strong>{item.name}</strong>
                  <StatusPill state={item.state}>{item.state}</StatusPill>
                  {item.managedByCutokyo ? (
                    <span className="owned-label">
                      <ShieldCheck aria-hidden="true" /> Cutokyo-owned
                    </span>
                  ) : null}
                </div>
                <p>{item.description}</p>
                <div className="inventory-row__meta">
                  <span>
                    <b>Scope</b> {item.scope}
                  </span>
                  <span>
                    <b>Origin</b> {item.origin}
                  </span>
                </div>
                <div
                  className="harness-chips"
                  role="group"
                  aria-label="Configured harnesses"
                >
                  {item.harnesses.map((harness) => (
                    <span key={harness}>{HARNESS_NAMES[harness]}</span>
                  ))}
                </div>
                <ProvenanceDetails
                  provenance={item.provenance}
                  label="Discovery provenance"
                />
              </div>
              <div className="inventory-row__action">
                {item.kind === "mcp" && item.id !== "mcp-cutokyo-search" ? (
                  <label className="switch-control">
                    <span>
                      {item.state === "enabled" ? "Enabled" : "Disabled"} for
                      all
                    </span>
                    <input
                      type="checkbox"
                      role="switch"
                      checked={item.state === "enabled"}
                      disabled={busyItem === item.id}
                      onChange={() => void toggleMcp(item)}
                      aria-label={`${item.state === "enabled" ? "Disable" : "Enable"} ${item.name} for all configured harnesses`}
                    />
                  </label>
                ) : item.kind === "plugin" ? (
                  <Button
                    size="small"
                    onClick={() => void openVerification(item.id)}
                  >
                    Verification <ChevronRight aria-hidden="true" />
                  </Button>
                ) : null}
              </div>
            </article>
          ))}
        </div>
      )}

      {verification === null ? null : (
        <Modal
          title="Plugin verification"
          description="Runtime protocol evidence and approved data capabilities."
          size="wide"
          onClose={() => setVerification(null)}
          footer={<Button onClick={() => setVerification(null)}>Done</Button>}
        >
          {verification === "loading" ? (
            <LoadingState label="Reading verifier evidence" />
          ) : (
            <div className="verification-grid">
              <DefinitionList
                rows={[
                  {
                    term: "Protocol",
                    value: `Major ${verification.protocolMajor}`,
                  },
                  {
                    term: "Compatibility",
                    value: (
                      <StatusPill
                        state={
                          verification.protocolState === "compatible"
                            ? "healthy"
                            : "unknown"
                        }
                      >
                        {verification.protocolState}
                      </StatusPill>
                    ),
                  },
                  {
                    term: "Transcript access",
                    value: verification.transcriptApproved
                      ? "Approved"
                      : "Not approved",
                  },
                  {
                    term: "Network access",
                    value: verification.networkApproved
                      ? "Approved"
                      : "Not approved",
                  },
                ]}
              />
              <section>
                <h3>Declared capabilities</h3>
                <ul className="check-list">
                  {verification.capabilities.map((value) => (
                    <li key={value}>{value}</li>
                  ))}
                </ul>
              </section>
              <section>
                <h3>Enforced bounds</h3>
                <DefinitionList
                  rows={verification.limits.map((limit) => ({
                    term: limit.label,
                    value: limit.value,
                  }))}
                />
              </section>
              <section>
                <h3>Fixture evidence</h3>
                <ul className="check-list">
                  {verification.evidence.map((value) => (
                    <li key={value}>{value}</li>
                  ))}
                </ul>
              </section>
              <Disclosure>{verification.sandboxDisclosure}</Disclosure>
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

import {
  Boxes,
  ChevronRight,
  Network,
  Puzzle,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
} from "lucide-react";
import { useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  Harness,
  InventoryKind,
  InventoryItem,
  PluginVerification,
} from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import { InventoryManager } from "../components/InventoryManager.js";
import { routeHref } from "../router.js";
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
  SuccessMessage,
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
  { id: "instruction", label: "Instructions" },
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
  const [query, setQuery] = useState("");
  const [harness, setHarness] = useState<Harness | "all">("all");
  const [selected, setSelected] = useState<InventoryItem | null>(null);
  const [receipt, setReceipt] = useState<string | null>(null);
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
  const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  const matching = data.items.filter((item) => {
    const text = [
      item.name,
      item.description,
      item.origin,
      item.kind,
      item.scope,
    ]
      .join(" ")
      .toLocaleLowerCase();
    return (
      (harness === "all" || item.harnesses.includes(harness)) &&
      terms.every((term) => text.includes(term))
    );
  });
  const visible =
    tab === "all" ? matching : matching.filter((item) => item.kind === tab);
  const complete = (message: string) => {
    setSelected(null);
    setReceipt(message);
    announce(message);
    resource.reload();
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
        eyebrow="Your local installations"
        title="Agent tools"
        description="Manage skills, MCP servers, hooks, plugins, and instructions. Edit their native configuration or install a copy in another supported harness."
        actions={
          <Button
            size="small"
            disabled={resource.state === "loading"}
            onClick={resource.reload}
          >
            <RefreshCw aria-hidden="true" />
            {resource.state === "loading"
              ? "Refreshing…"
              : "Refresh installations"}
          </Button>
        }
      />
      <RouteNotice meta={data.meta} />
      {resource.state === "error" ? (
        <ErrorState
          title="Installations could not be refreshed"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {receipt === null ? null : <SuccessMessage>{receipt}</SuccessMessage>}
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <section className="broker-summary" aria-labelledby="broker-heading">
        <Network aria-hidden="true" />
        <div>
          <h2 id="broker-heading">Cutokyo search access</h2>
          <p>
            Let your agents search local session history through Cutokyo's
            read-only MCP. Native MCP servers are managed individually below.
          </p>
          <a href={routeHref("/settings")}>Manage search access in Settings</a>
        </div>
        <StatusPill state={data.searchMcpEnabled ? "enabled" : "disabled"}>
          Search MCP {data.searchMcpEnabled ? "enabled" : "disabled"}
        </StatusPill>
      </section>

      <section className="inventory-filters" aria-label="Agent tool filters">
        <label className="search-field">
          <span className="sr-only">Search installed tools</span>
          <Search aria-hidden="true" />
          <input
            type="search"
            aria-label="Search installed tools"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Find a skill, server, hook, or instruction…"
          />
        </label>
        <label className="field-stack">
          <span>Harness</span>
          <select
            value={harness}
            onChange={(event) =>
              setHarness(event.currentTarget.value as Harness | "all")
            }
          >
            <option value="all">All harnesses</option>
            {Object.entries(HARNESS_NAMES).map(([id, name]) => (
              <option value={id} key={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        {query !== "" || harness !== "all" || tab !== "all" ? (
          <Button
            variant="quiet"
            size="small"
            onClick={() => {
              setQuery("");
              setHarness("all");
              setTab("all");
            }}
          >
            Clear filters
          </Button>
        ) : null}
      </section>

      <div className="tabs" role="tablist" aria-label="Inventory type">
        {TABS.map((item) => (
          <button
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            id={`inventory-tab-${item.id}`}
            aria-controls="inventory-results"
            tabIndex={tab === item.id ? 0 : -1}
            onKeyDown={(event) => {
              const index = TABS.findIndex((entry) => entry.id === tab);
              const next =
                event.key === "ArrowRight"
                  ? (index + 1) % TABS.length
                  : event.key === "ArrowLeft"
                    ? (index + TABS.length - 1) % TABS.length
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? TABS.length - 1
                        : null;
              if (next === null) return;
              event.preventDefault();
              const entry = TABS[next];
              if (entry === undefined) return;
              setTab(entry.id);
              document.getElementById(`inventory-tab-${entry.id}`)?.focus();
            }}
            className={tab === item.id ? "tab is-active" : "tab"}
            onClick={() => setTab(item.id)}
            key={item.id}
          >
            {item.label}
            <span>
              {item.id === "all"
                ? matching.length
                : matching.filter((entry) => entry.kind === item.id).length}
            </span>
          </button>
        ))}
      </div>

      <section
        id="inventory-results"
        role="tabpanel"
        aria-labelledby={`inventory-tab-${tab}`}
      >
        <div className="results-heading" role="status">
          <strong>
            {visible.length}{" "}
            {visible.length === 1 ? "installation" : "installations"}
          </strong>
          <span>
            {resource.state === "loading"
              ? "Refreshing native files…"
              : "Select Manage to view or change an installation"}
          </span>
        </div>
        {visible.length === 0 ? (
          <EmptyState
            compact
            title={
              data.items.length === 0
                ? "No agent tools found"
                : "No installations match these filters"
            }
            description={
              data.items.length === 0
                ? "Install a skill or configure an MCP server in your harness, then refresh. Check capture setup if a harness is unavailable."
                : "Try another harness or tool type, or clear your search."
            }
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
                  <Button
                    size="small"
                    aria-label={`Manage ${item.name}`}
                    onClick={() => {
                      setError(null);
                      setReceipt(null);
                      setSelected(item);
                    }}
                  >
                    <Settings2 aria-hidden="true" /> Manage
                  </Button>
                  {item.kind === "plugin" ? (
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
      </section>

      {selected === null ? null : (
        <InventoryManager
          key={selected.id}
          item={selected}
          onClose={() => setSelected(null)}
          onComplete={complete}
        />
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

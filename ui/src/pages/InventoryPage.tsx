import {
  ChevronRight,
  Link2,
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
import { HarnessFilter, HarnessLabel } from "../components/HarnessMark.js";
import { InventoryManager } from "../components/InventoryManager.js";
import { KIND_LABEL, KindIcon } from "../components/KindIcon.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  EmptyState,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  StatusPill,
  SuccessMessage,
} from "../components/Primitives.js";
import { RouteNotice } from "../components/Notices.js";

const SECTIONS: readonly {
  readonly kind: InventoryKind;
  readonly label: string;
}[] = [
  { kind: "skill", label: "Skills" },
  { kind: "mcp", label: "MCP servers" },
  { kind: "hook", label: "Hooks" },
  { kind: "plugin", label: "Plugins" },
  { kind: "instruction", label: "Instructions" },
];

const ROUTINE_STATES = new Set(["enabled", "installed", "configured"]);

export function InventoryPage() {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getInventory(),
    "inventory",
  );
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"all" | InventoryKind>("all");
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
    kind === "all" ? matching : matching.filter((item) => item.kind === kind);
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
        title="Agent tools"
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

      <section className="toolbar" aria-label="Agent tool filters">
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
        <div className="segmented" role="group" aria-label="Tool type">
          {(
            [
              { id: "all", label: "All" },
              ...SECTIONS.map((entry) => ({
                id: entry.kind,
                label: entry.label,
              })),
            ] as const
          ).map((entry) => (
            <button
              type="button"
              key={entry.id}
              className="segmented__item"
              aria-pressed={kind === entry.id}
              onClick={() => setKind(entry.id)}
            >
              {entry.label}
              <span className="count-badge">
                {entry.id === "all"
                  ? matching.length
                  : matching.filter((item) => item.kind === entry.id).length}
              </span>
            </button>
          ))}
        </div>
        <HarnessFilter value={harness} onChange={setHarness} />
        {query !== "" || harness !== "all" || kind !== "all" ? (
          <Button
            variant="quiet"
            onClick={() => {
              setQuery("");
              setHarness("all");
              setKind("all");
            }}
          >
            Clear filters
          </Button>
        ) : null}
      </section>

      <section id="inventory-results" aria-label="Installed agent tools">
        <p className="sr-only" role="status">
          {visible.length}{" "}
          {visible.length === 1 ? "installation" : "installations"}
        </p>
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
                ? "Install a skill or MCP server in your agent, then refresh."
                : "Clear the filters to see everything."
            }
          />
        ) : (
          <div className="inventory-sections">
            {SECTIONS.map((section) => {
              const items = visible.filter(
                (item) => item.kind === section.kind,
              );
              if (items.length === 0) return null;
              const headingId = `inventory-section-${section.kind}`;
              return (
                <section key={section.kind} aria-labelledby={headingId}>
                  <h2 className="section-heading" id={headingId}>
                    {section.label}
                    <span className="count-badge">{items.length}</span>
                  </h2>
                  <div className="inventory-list">
                    {items.map((item) => (
                      <article className="inventory-card" key={item.id}>
                        <div
                          className={`inventory-row__icon inventory-row__icon--${item.kind}`}
                          role="img"
                          aria-label={KIND_LABEL[item.kind]}
                          title={KIND_LABEL[item.kind]}
                        >
                          <KindIcon kind={item.kind} />
                        </div>
                        <div className="inventory-card__body">
                          <div className="inventory-row__title">
                            <strong>{item.name}</strong>
                            {ROUTINE_STATES.has(item.state) ? null : (
                              <span
                                className={`quiet-badge quiet-badge--${item.state}`}
                              >
                                {item.state}
                              </span>
                            )}
                            {item.managedByCutokyo ? (
                              <span className="owned-label">
                                <ShieldCheck aria-hidden="true" />
                                Cutokyo-owned
                              </span>
                            ) : null}
                          </div>
                          <p
                            className="inventory-row__description"
                            title={item.description}
                          >
                            {item.description}
                          </p>
                          <div className="inventory-card__footer">
                            <div
                              className="harness-chips harness-chips--start"
                              role="group"
                              aria-label="Configured harnesses"
                            >
                              {item.harnesses.map((entry) => (
                                <HarnessLabel key={entry} harness={entry} />
                              ))}
                            </div>
                            <span
                              className="inventory-row__scope"
                              title={item.origin}
                            >
                              {item.scope}
                              {item.linkTarget == null ? null : (
                                <>
                                  {" · "}
                                  <Link2 aria-hidden="true" /> linked
                                </>
                              )}
                            </span>
                          </div>
                        </div>
                        <div className="inventory-row__action">
                          {item.kind === "plugin" ? (
                            <Button
                              size="small"
                              onClick={() => void openVerification(item.id)}
                            >
                              Verification <ChevronRight aria-hidden="true" />
                            </Button>
                          ) : null}
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
                        </div>
                      </article>
                    ))}
                  </div>
                </section>
              );
            })}
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
          size="wide"
          onClose={() => setVerification(null)}
          footer={<Button onClick={() => setVerification(null)}>Done</Button>}
        >
          {verification === "loading" ? (
            <LoadingState label="Loading" />
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
                <h3>Checks</h3>
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

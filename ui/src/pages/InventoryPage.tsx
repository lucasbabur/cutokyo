import {
  AlertTriangle,
  LayoutGrid,
  Plus,
  RefreshCw,
  Search,
} from "lucide-react";
import { useId, useRef, useState, type ReactNode } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { Harness, InventoryItem, InventoryKind } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  HARNESS_NAMES,
  HARNESS_ORDER,
  HarnessMark,
} from "../components/HarnessMark.js";
import {
  InventoryManager,
  type InstallRequest,
} from "../components/InventoryManager.js";
import { KIND_LABEL, KindIcon } from "../components/KindIcon.js";
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
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
/** Mirrors the core install planner: hooks and plugins never convert. */
const PORTABLE_KINDS = new Set<InventoryKind>(["skill", "mcp", "instruction"]);

type Filter =
  "all" | "attention" | "gaps" | InventoryKind | `harness:${Harness}`;

/** Every installation of one tool (same kind and name) across harnesses. */
interface Tool {
  readonly key: string;
  readonly kind: InventoryKind;
  readonly name: string;
  readonly items: readonly InventoryItem[];
}

const harnessesOf = (tool: Tool) =>
  new Set(tool.items.flatMap((item) => item.harnesses));
const needsAttention = (tool: Tool) =>
  tool.items.some((item) => !ROUTINE_STATES.has(item.state));
const portable = (tool: Tool) =>
  PORTABLE_KINDS.has(tool.kind) &&
  tool.items.some((item) => !item.managedByCutokyo);
const hasGap = (tool: Tool) =>
  portable(tool) && harnessesOf(tool).size < HARNESS_ORDER.length;

function groupTools(items: readonly InventoryItem[]): Tool[] {
  const tools = new Map<string, InventoryItem[]>();
  for (const item of items) {
    const key = `${item.kind}:${item.name}`;
    tools.set(key, [...(tools.get(key) ?? []), item]);
  }
  return [...tools.entries()]
    .map(([key, entries]) => ({
      key,
      kind: entries[0]!.kind,
      name: entries[0]!.name,
      items: entries,
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function matchesFilter(tool: Tool, filter: Filter): boolean {
  if (filter === "all") return true;
  if (filter === "attention") return needsAttention(tool);
  if (filter === "gaps") return hasGap(tool);
  if (filter.startsWith("harness:")) {
    return harnessesOf(tool).has(filter.slice("harness:".length) as Harness);
  }
  return tool.kind === filter;
}

export function InventoryPage() {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getInventory(),
    "inventory",
  );
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [installRequest, setInstallRequest] = useState<InstallRequest | null>(
    null,
  );
  const [receipt, setReceipt] = useState<string | null>(null);
  const announce = useAnnounce();
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());

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
  const searched = groupTools(
    data.items.filter((item) => {
      const text = [item.name, item.description, item.origin, item.scope]
        .join(" ")
        .toLocaleLowerCase();
      return terms.every((term) => text.includes(term));
    }),
  );
  const visible = searched.filter((tool) => matchesFilter(tool, filter));
  const selected =
    groupTools(data.items).find((tool) => tool.key === selectedKey) ?? null;
  const selectedItem =
    selected?.items.find((item) => item.id === selectedId) ??
    selected?.items[0] ??
    null;

  const select = (tool: Tool, item?: InventoryItem) => {
    setReceipt(null);
    setInstallRequest(null);
    setSelectedKey(tool.key);
    setSelectedId((item ?? tool.items[0]!).id);
  };
  const complete = (message: string) => {
    setInstallRequest(null);
    setReceipt(message);
    announce(message);
    resource.reload();
  };
  const count = (predicate: (tool: Tool) => boolean) =>
    searched.filter(predicate).length;
  const attention = count(needsAttention);

  const railButton = (
    id: Filter,
    label: string,
    icon: ReactNode,
    total: number,
    className = "",
  ) => (
    <button
      type="button"
      key={id}
      className={`tool-rail__item ${className}`}
      aria-pressed={filter === id}
      onClick={() => setFilter(id)}
    >
      {icon}
      <span className="tool-rail__label">{label}</span>
      <span className="tool-rail__count">{total}</span>
    </button>
  );

  return (
    <div className="page page--tools">
      <header className="tools-header">
        <h1 tabIndex={-1} data-page-heading>
          Agent tools
        </h1>
        <label className="search-field tools-header__search">
          <Search aria-hidden="true" />
          <input
            type="search"
            aria-label="Search installed tools"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search tools"
          />
        </label>
        <Button
          size="small"
          aria-label="Rescan installations"
          title="Rescan installations"
          disabled={resource.state === "loading"}
          onClick={resource.reload}
        >
          <RefreshCw aria-hidden="true" />
        </Button>
      </header>
      <RouteNotice meta={data.meta} />
      {resource.state === "error" ? (
        <ErrorState
          title="Installations could not be refreshed"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {receipt === null ? null : <SuccessMessage>{receipt}</SuccessMessage>}

      <div className={`tools-layout${selected ? " has-panel" : ""}`}>
        <nav className="tool-rail" aria-label="Tool filters">
          {railButton(
            "all",
            "All tools",
            <LayoutGrid aria-hidden="true" />,
            searched.length,
          )}
          {attention === 0
            ? null
            : railButton(
                "attention",
                "Needs attention",
                <AlertTriangle aria-hidden="true" />,
                attention,
                "tool-rail__item--attention",
              )}
          <h2 className="tool-rail__heading">Type</h2>
          {SECTIONS.map((section) =>
            railButton(
              section.kind,
              section.label,
              <KindIcon kind={section.kind} />,
              count((tool) => tool.kind === section.kind),
            ),
          )}
          <h2 className="tool-rail__heading">Harness</h2>
          {HARNESS_ORDER.map((harness) =>
            railButton(
              `harness:${harness}`,
              HARNESS_NAMES[harness],
              <HarnessMark harness={harness} />,
              count((tool) => harnessesOf(tool).has(harness)),
            ),
          )}
          <h2 className="tool-rail__heading">Coverage</h2>
          {railButton(
            "gaps",
            "Not everywhere",
            <Plus aria-hidden="true" />,
            count(hasGap),
          )}
        </nav>

        <section className="tool-list" aria-label="Agent tools">
          <p className="sr-only" role="status">
            {visible.length} {visible.length === 1 ? "tool" : "tools"}
          </p>
          {visible.length === 0 ? (
            <EmptyState
              compact
              title={
                data.items.length === 0
                  ? "No agent tools found"
                  : "No tools match"
              }
              description={
                data.items.length === 0
                  ? "Install a skill or MCP server in your agent, then rescan."
                  : "Change the search or pick another filter."
              }
            />
          ) : (
            <table className="tool-table">
              <colgroup>
                <col />
                {HARNESS_ORDER.map((harness) => (
                  <col key={harness} className="tool-table__harness-col" />
                ))}
              </colgroup>
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  {HARNESS_ORDER.map((harness) => (
                    <th
                      scope="col"
                      key={harness}
                      className="tool-table__harness"
                    >
                      <span>
                        <HarnessMark harness={harness} />
                        <span className="tool-table__harness-name">
                          {HARNESS_NAMES[harness]}
                        </span>
                      </span>
                    </th>
                  ))}
                </tr>
              </thead>
              {SECTIONS.map((section) => {
                const tools = visible.filter(
                  (tool) => tool.kind === section.kind,
                );
                if (tools.length === 0) return null;
                return (
                  <tbody key={section.kind}>
                    <tr className="tool-table__group">
                      <th scope="colgroup" colSpan={1 + HARNESS_ORDER.length}>
                        {section.label}
                        <span>{tools.length}</span>
                      </th>
                    </tr>
                    {tools.map((tool) => (
                      <ToolRow
                        key={tool.key}
                        tool={tool}
                        selected={tool.key === selected?.key}
                        buttonRef={(button) => {
                          if (button === null) {
                            rowButtons.current.delete(tool.key);
                          } else rowButtons.current.set(tool.key, button);
                        }}
                        onSelect={() => select(tool)}
                        onInstall={(harness) => {
                          select(tool);
                          setInstallRequest({ harness, nonce: Date.now() });
                        }}
                      />
                    ))}
                  </tbody>
                );
              })}
            </table>
          )}
        </section>

        {selected === null || selectedItem === null ? null : (
          <InventoryManager
            key={selectedItem.id}
            item={selectedItem}
            installations={selected.items}
            installRequest={installRequest}
            onSelect={(item) => select(selected, item)}
            onClose={() => {
              rowButtons.current.get(selected.key)?.focus();
              setSelectedKey(null);
              setInstallRequest(null);
            }}
            onComplete={complete}
          />
        )}
      </div>
    </div>
  );
}

function ToolRow({
  tool,
  selected,
  buttonRef,
  onSelect,
  onInstall,
}: {
  readonly tool: Tool;
  readonly selected: boolean;
  readonly buttonRef: (button: HTMLButtonElement | null) => void;
  readonly onSelect: () => void;
  readonly onInstall: (harness: Harness) => void;
}) {
  const descriptionId = useId();
  const installed = harnessesOf(tool);
  const first = tool.items[0]!;
  const unusual = tool.items.find((item) => !ROUTINE_STATES.has(item.state));
  const scopes = [...new Set(tool.items.map((item) => item.scope))].filter(
    (scope) => scope !== "user",
  );
  return (
    <tr
      className="tool-row"
      aria-selected={selected}
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("button")) return;
        onSelect();
      }}
    >
      <td>
        <button
          type="button"
          ref={buttonRef}
          className="tool-row__name"
          aria-label={tool.name}
          aria-describedby={descriptionId}
          aria-current={selected ? "true" : undefined}
          onClick={onSelect}
        >
          <span
            className={`tool-row__icon tool-row__icon--${tool.kind}`}
            role="img"
            aria-label={KIND_LABEL[tool.kind]}
          >
            <KindIcon kind={tool.kind} />
          </span>
          <span className="tool-row__text">
            <span className="tool-row__title">
              <strong>{tool.name}</strong>
              {scopes.map((scope) => (
                <span key={scope} className="tool-row__scope">
                  {scope}
                </span>
              ))}
              {first.managedByCutokyo ? (
                <span className="tool-row__owned">Cutokyo</span>
              ) : null}
              {unusual === undefined ? null : (
                <span className={`quiet-badge quiet-badge--${unusual.state}`}>
                  {unusual.state}
                </span>
              )}
            </span>
            <span className="tool-row__description" id={descriptionId}>
              {first.description}
            </span>
          </span>
        </button>
      </td>
      {HARNESS_ORDER.map((harness) => (
        <td key={harness} className="tool-row__cell">
          {installed.has(harness) ? (
            <span
              className={`tool-dot tool-dot--${harness}`}
              role="img"
              aria-label={`Installed in ${HARNESS_NAMES[harness]}`}
            />
          ) : portable(tool) ? (
            <button
              type="button"
              className="tool-row__install"
              aria-label={`Install ${tool.name} in ${HARNESS_NAMES[harness]}`}
              title={`Install in ${HARNESS_NAMES[harness]}`}
              onClick={() => onInstall(harness)}
            >
              <Plus aria-hidden="true" />
            </button>
          ) : (
            <span
              className="tool-row__none"
              role="img"
              aria-label={`Not available in ${HARNESS_NAMES[harness]}`}
            />
          )}
        </td>
      ))}
    </tr>
  );
}

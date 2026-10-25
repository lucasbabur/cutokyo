import { ArrowLeft, ExternalLink, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  ToolCallDateRange,
  ToolCallFilters,
  ToolCallOutcome,
  ToolCallSample,
  ToolCallSamplesResponse,
  ToolCallStat,
} from "../contracts.js";
import {
  HARNESS_NAMES,
  HarnessFilter,
  HarnessMark,
} from "../components/HarnessMark.js";
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
  PageHeader,
  formatDateTime,
} from "../components/Primitives.js";
import { RouteNotice } from "../components/Notices.js";
import { formatInteger } from "../domain/reconcile.js";
import { routeHref } from "../router.js";

const SAMPLE_PAGE = 25;
const FILTERS_KEY = "cutokyo.tool-call-filters";
const DEFAULT_FILTERS: ToolCallFilters = {
  harness: "all",
  dateRange: "all",
  todayStart: null,
};
const DATE_RANGES: readonly ToolCallDateRange[] = [
  "all",
  "today",
  "7d",
  "30d",
  "90d",
];

type ToolSort = "calls" | "failures" | "success" | "name";

function retainedFilters(): ToolCallFilters {
  try {
    const saved: unknown = JSON.parse(
      sessionStorage.getItem(FILTERS_KEY) ?? "null",
    );
    if (saved === null || typeof saved !== "object") return DEFAULT_FILTERS;
    const fields = saved as Record<string, unknown>;
    return {
      ...DEFAULT_FILTERS,
      harness: ["all", "claude_code", "codex", "opencode"].includes(
        String(fields.harness),
      )
        ? (fields.harness as ToolCallFilters["harness"])
        : "all",
      dateRange: DATE_RANGES.includes(fields.dateRange as ToolCallDateRange)
        ? (fields.dateRange as ToolCallDateRange)
        : "all",
    };
  } catch {
    return DEFAULT_FILTERS;
  }
}

/** Harness and date filters survive moving between the list and a tool. */
function useToolCallFilters() {
  const [filters, setFilters] = useState(retainedFilters);
  useEffect(() => {
    try {
      sessionStorage.setItem(FILTERS_KEY, JSON.stringify(filters));
    } catch {
      /* Storage can be disabled by the host. */
    }
  }, [filters]);
  return [filters, setFilters] as const;
}

function useToolCallStats(filters: ToolCallFilters) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getToolCallStats(filters),
    JSON.stringify(filters),
  );
  useEffect(
    () => commands.onHistoryImported(resource.reload),
    [commands, resource.reload],
  );
  return resource;
}

/** Rounds without ever showing 100% or 0% for a mix of outcomes. */
export function percent(part: number, total: number): string {
  if (total === 0) return "—";
  if (part === total) return "100%";
  if (part === 0) return "0%";
  const value = (part / total) * 100;
  if (value >= 99.95) return ">99.9%";
  if (value < 0.05) return "<0.1%";
  return `${value.toFixed(1)}%`;
}

function FilterBar({
  filters,
  onChange,
}: {
  readonly filters: ToolCallFilters;
  readonly onChange: (next: ToolCallFilters) => void;
}) {
  return (
    <>
      <HarnessFilter
        value={filters.harness}
        onChange={(harness) => onChange({ ...filters, harness })}
      />
      <label className="inline-field">
        <span>Date</span>
        <select
          aria-label="Date"
          value={filters.dateRange}
          onChange={(event) =>
            onChange({
              ...filters,
              dateRange: event.currentTarget.value as ToolCallDateRange,
            })
          }
        >
          <option value="all">Any time</option>
          <option value="today">Today</option>
          <option value="7d">Last 7 days</option>
          <option value="30d">Last 30 days</option>
          <option value="90d">Last 90 days</option>
        </select>
      </label>
    </>
  );
}

function OutcomeBar({ stat }: { readonly stat: ToolCallStat }) {
  const width = (value: number) =>
    stat.total === 0 ? "0%" : `${(value / stat.total) * 100}%`;
  return (
    <span className="outcome-bar" aria-hidden="true">
      <span
        className="outcome-bar__succeeded"
        style={{ width: width(stat.succeeded) }}
      />
      <span
        className="outcome-bar__failed"
        style={{ width: width(stat.failed) }}
      />
    </span>
  );
}

export function ToolCallsPage({
  toolName,
}: {
  readonly toolName: string | null;
}) {
  const [filters, setFilters] = useToolCallFilters();
  return toolName === null ? (
    <ToolCallList filters={filters} onFiltersChange={setFilters} />
  ) : (
    <ToolCallDetail
      key={toolName}
      toolName={toolName}
      filters={filters}
      onFiltersChange={setFilters}
    />
  );
}

function ToolCallList({
  filters,
  onFiltersChange,
}: {
  readonly filters: ToolCallFilters;
  readonly onFiltersChange: (next: ToolCallFilters) => void;
}) {
  const resource = useToolCallStats(filters);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<ToolSort>("calls");
  const tools = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matching = (resource.data?.tools ?? []).filter((tool) =>
      tool.toolName.toLowerCase().includes(needle),
    );
    const successRate = (tool: ToolCallStat) =>
      tool.total === 0 ? 1 : tool.succeeded / tool.total;
    return [...matching].sort((left, right) =>
      sort === "name"
        ? left.toolName.localeCompare(right.toolName)
        : sort === "failures"
          ? right.failed - left.failed || right.total - left.total
          : sort === "success"
            ? successRate(left) - successRate(right) || right.total - left.total
            : right.total - left.total,
    );
  }, [query, resource.data, sort]);
  const totals = (resource.data?.tools ?? []).reduce(
    (sum, tool) => ({
      total: sum.total + tool.total,
      succeeded: sum.succeeded + tool.succeeded,
      failed: sum.failed + tool.failed,
    }),
    { total: 0, succeeded: 0, failed: 0 },
  );

  return (
    <div className="page">
      <PageHeader title="Tool calls" />
      <section className="toolbar toolbar--wrap" aria-label="Tool call filters">
        <label className="search-field">
          <span className="sr-only">Filter tools by name</span>
          <Search aria-hidden="true" />
          <input
            type="search"
            aria-label="Filter tools by name"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Filter tools…"
          />
        </label>
        <FilterBar filters={filters} onChange={onFiltersChange} />
        <label className="inline-field">
          <span>Sort</span>
          <select
            aria-label="Sort"
            value={sort}
            onChange={(event) => setSort(event.currentTarget.value as ToolSort)}
          >
            <option value="calls">Most calls</option>
            <option value="failures">Most failures</option>
            <option value="success">Lowest success rate</option>
            <option value="name">Name</option>
          </select>
        </label>
      </section>

      {resource.state === "error" ? (
        <ErrorState
          title="Tool calls are unavailable"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {resource.data === null ? (
        resource.state === "loading" ? (
          <LoadingState label="Counting tool calls" />
        ) : null
      ) : (
        <>
          <RouteNotice meta={resource.data.meta} />
          {resource.data.tools.length === 0 ? (
            <EmptyState
              title="No tool calls"
              description="No captured tool calls match these filters."
            />
          ) : (
            <section className="panel" aria-labelledby="tool-table-heading">
              <div className="panel__header">
                <h2 id="tool-table-heading">
                  {formatInteger(totals.total)} calls ·{" "}
                  {percent(totals.succeeded, totals.total)} succeeded ·{" "}
                  {formatInteger(totals.failed)} failed
                </h2>
              </div>
              {tools.length === 0 ? (
                <EmptyState
                  compact
                  title="No matching tools"
                  description={`No tool name contains “${query.trim()}”.`}
                />
              ) : (
                <div className="tool-table-scroll">
                  <table className="data-table tool-table">
                    <thead>
                      <tr>
                        <th scope="col">Tool</th>
                        <th scope="col">Harness</th>
                        <th scope="col" className="numeric">
                          Calls
                        </th>
                        <th scope="col">Success</th>
                        <th scope="col" className="numeric">
                          Failed
                        </th>
                        <th scope="col">Last called</th>
                      </tr>
                    </thead>
                    <tbody>
                      {tools.map((tool) => (
                        <ToolRow key={tool.toolName} tool={tool} />
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}

function ToolRow({ tool }: { readonly tool: ToolCallStat }) {
  const href = routeHref("/tools", tool.toolName);
  return (
    <tr
      className="tool-table__row"
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("a") === null)
          globalThis.location.hash = href.slice(1);
      }}
    >
      <th scope="row">
        <a className="tool-table__name" href={href}>
          {tool.toolName}
        </a>
      </th>
      <td>
        <span className="tool-table__harnesses">
          {tool.harnesses.map((harness) => (
            <span key={harness} title={HARNESS_NAMES[harness]}>
              <HarnessMark harness={harness} />
              <span className="sr-only">{HARNESS_NAMES[harness]}</span>
            </span>
          ))}
        </span>
      </td>
      <td className="numeric">{formatInteger(tool.total)}</td>
      <td>
        <span className="tool-table__rate">
          <OutcomeBar stat={tool} />
          <span className="numeric">{percent(tool.succeeded, tool.total)}</span>
        </span>
      </td>
      <td
        className={tool.failed > 0 ? "numeric tool-table__failed" : "numeric"}
      >
        {formatInteger(tool.failed)}
      </td>
      <td>
        <time dateTime={tool.lastCalledAt}>
          {formatDateTime(tool.lastCalledAt)}
        </time>
      </td>
    </tr>
  );
}

function ToolCallDetail({
  toolName,
  filters,
  onFiltersChange,
}: {
  readonly toolName: string;
  readonly filters: ToolCallFilters;
  readonly onFiltersChange: (next: ToolCallFilters) => void;
}) {
  const stats = useToolCallStats(filters);
  const stat =
    stats.data?.tools.find((tool) => tool.toolName === toolName) ?? null;
  const [chosenOutcome, setChosenOutcome] = useState<ToolCallOutcome | null>(
    null,
  );
  // Failures are what people come here for, so they open first when present.
  const outcome: ToolCallOutcome =
    chosenOutcome ?? ((stat?.failed ?? 0) > 0 ? "failed" : "all");

  return (
    <div className="page">
      <a className="back-link" href={routeHref("/tools")}>
        <ArrowLeft aria-hidden="true" /> Back to tool calls
      </a>
      <PageHeader title={toolName} />
      <section className="toolbar toolbar--wrap" aria-label="Tool call filters">
        <div className="segmented" role="group" aria-label="Outcome">
          {(
            [
              ["all", "All", stat?.total],
              ["failed", "Failed", stat?.failed],
              ["succeeded", "Succeeded", stat?.succeeded],
            ] as const
          ).map(([value, label, count]) => (
            <button
              type="button"
              key={value}
              className="segmented__item"
              aria-pressed={outcome === value}
              onClick={() => setChosenOutcome(value)}
            >
              {label}
              {count === undefined ? null : (
                <span className="segmented__count">{formatInteger(count)}</span>
              )}
            </button>
          ))}
        </div>
        <FilterBar filters={filters} onChange={onFiltersChange} />
      </section>

      {stats.state === "error" ? (
        <ErrorState
          title="Tool call counts are unavailable"
          error={stats.error}
          onRetry={stats.reload}
        />
      ) : null}
      {stats.data === null ? (
        stats.state === "loading" ? (
          <LoadingState label="Counting tool calls" />
        ) : null
      ) : stat === null ? (
        <EmptyState
          title="No calls"
          description={`No captured ${toolName} calls match these filters.`}
        />
      ) : (
        <>
          <dl className="tool-stats">
            <div>
              <dt>Calls</dt>
              <dd>{formatInteger(stat.total)}</dd>
            </div>
            <div>
              <dt>Succeeded</dt>
              <dd>{percent(stat.succeeded, stat.total)}</dd>
            </div>
            <div className={stat.failed > 0 ? "tool-stats__failed" : ""}>
              <dt>Failed</dt>
              <dd>{percent(stat.failed, stat.total)}</dd>
            </div>
            {stat.other > 0 ? (
              <div>
                <dt>No outcome</dt>
                <dd>{formatInteger(stat.other)}</dd>
              </div>
            ) : null}
            <div>
              <dt>Last called</dt>
              <dd>{formatDateTime(stat.lastCalledAt)}</dd>
            </div>
          </dl>
          <SampleList
            key={`${outcome}:${JSON.stringify(filters)}`}
            toolName={toolName}
            outcome={outcome}
            filters={filters}
          />
        </>
      )}
    </div>
  );
}

function SampleList({
  toolName,
  outcome,
  filters,
}: {
  readonly toolName: string;
  readonly outcome: ToolCallOutcome;
  readonly filters: ToolCallFilters;
}) {
  const commands = useCommands();
  const [pages, setPages] = useState<readonly ToolCallSamplesResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const [attempt, setAttempt] = useState(0);
  // The next page starts where the loaded ones end; retries reuse the offset.
  const [requestOffset, setRequestOffset] = useState(0);

  useEffect(() => {
    let current = true;
    setLoading(true);
    setError(null);
    commands
      .getToolCallSamples({
        toolName,
        outcome,
        filters,
        offset: requestOffset,
        limit: SAMPLE_PAGE,
      })
      .then((page) => {
        if (current) setPages((previous) => [...previous, page]);
      })
      .catch((reason: unknown) => {
        if (current)
          setError(
            reason instanceof Error ? reason : new Error(String(reason)),
          );
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [attempt, commands, filters, outcome, requestOffset, toolName]);

  const samples = pages.flatMap((page) => page.samples);
  const last = pages.at(-1);
  return (
    <section className="panel" aria-labelledby="samples-heading">
      <div className="panel__header">
        <h2 id="samples-heading">
          {last === undefined
            ? "Calls"
            : `${formatInteger(last.total)} ${outcome === "all" ? "" : `${outcome} `}calls, newest first`}
        </h2>
      </div>
      {error === null ? null : (
        <ErrorState
          title="Calls could not be read"
          error={error}
          onRetry={() => setAttempt((value) => value + 1)}
        />
      )}
      {samples.length === 0 && !loading && error === null ? (
        <EmptyState
          compact
          title="No calls"
          description="No calls match this outcome and these filters."
        />
      ) : (
        <ol className="tool-samples">
          {samples.map((sample) => (
            <li key={sample.id}>
              <SampleCard sample={sample} />
            </li>
          ))}
        </ol>
      )}
      {loading ? (
        <LoadingState label="Reading calls" />
      ) : last?.hasMore === true ? (
        <div className="tool-samples__more">
          <Button onClick={() => setRequestOffset(samples.length)}>
            Load {Math.min(SAMPLE_PAGE, last.total - samples.length)} more
          </Button>
        </div>
      ) : null}
    </section>
  );
}

/** A one-line hint of what the call did, taken from its input. */
export function inputPreview(input: unknown): string {
  if (input === null || input === undefined) return "No input recorded";
  if (typeof input === "string") return input;
  if (typeof input !== "object") return JSON.stringify(input);
  const fields = input as Record<string, unknown>;
  for (const name of [
    "command",
    "cmd",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "description",
    "prompt",
  ]) {
    const value = fields[name];
    if (typeof value === "string" && value.length > 0) return value;
    if (Array.isArray(value) && value.length > 0)
      return value.map(String).join(" ");
  }
  return JSON.stringify(input);
}

export function payloadText(value: unknown): string {
  if (value === null || value === undefined) return "";
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function duration(sample: ToolCallSample): string | null {
  if (sample.endedAt === null) return null;
  const milliseconds =
    Date.parse(sample.endedAt) - Date.parse(sample.startedAt);
  if (!Number.isFinite(milliseconds) || milliseconds <= 0) return null;
  return milliseconds < 1000
    ? `${milliseconds} ms`
    : milliseconds < 60_000
      ? `${(milliseconds / 1000).toFixed(1)} s`
      : `${Math.round(milliseconds / 60_000)} min`;
}

const STATE_LABELS: Readonly<Record<ToolCallSample["state"], string>> = {
  succeeded: "Succeeded",
  failed: "Failed",
  running: "Running",
  cancelled: "Cancelled",
  unknown: "No outcome",
};

function SampleCard({ sample }: { readonly sample: ToolCallSample }) {
  const took = duration(sample);
  const output = payloadText(sample.output);
  return (
    <details className={`tool-sample tool-sample--${sample.state}`}>
      <summary>
        {/* WebKitGTK ignores flex layout on <summary> itself. */}
        <span className="tool-sample__summary">
          <span className={`outcome-badge outcome-badge--${sample.state}`}>
            {STATE_LABELS[sample.state]}
          </span>
          <code className="tool-sample__preview">
            {inputPreview(sample.input).slice(0, 240)}
          </code>
          <span className="tool-sample__meta">
            <HarnessMark harness={sample.harness} />
            <span className="sr-only">{HARNESS_NAMES[sample.harness]}</span>
            <time dateTime={sample.startedAt}>
              {formatDateTime(sample.startedAt)}
            </time>
            {took === null ? null : <span>{took}</span>}
          </span>
        </span>
      </summary>
      <div className="tool-sample__body">
        <h3>Input</h3>
        <pre>{payloadText(sample.input) || "No input recorded"}</pre>
        <h3>Output</h3>
        <pre>{output || "No output recorded"}</pre>
        <a
          className="tool-sample__session"
          href={routeHref("/sessions", sample.sessionId)}
        >
          <ExternalLink aria-hidden="true" />
          {sample.sessionTitle ?? "Untitled session"}
        </a>
      </div>
    </details>
  );
}

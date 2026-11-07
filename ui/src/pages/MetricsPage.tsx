import { useEffect } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  Harness,
  HarnessMetrics,
  SetupMetrics,
  SetupMetricsResponse,
  ToolCallFilters,
} from "../contracts.js";
import { HARNESS_NAMES, HarnessLabel } from "../components/HarnessMark.js";
import {
  EmptyState,
  ErrorState,
  LoadingState,
  PageHeader,
} from "../components/Primitives.js";
import { RouteNotice } from "../components/Notices.js";
import {
  formatCompact,
  formatInteger,
  formatMicros,
} from "../domain/reconcile.js";
import { routeHref } from "../router.js";
import {
  FilterBar,
  ToolCallDetail,
  ToolCallsPanel,
  percent,
  useToolCallFilters,
} from "./ToolCalls.js";

/** A ratio of two known sums, or null when nothing in view records it. */
interface Ratio {
  readonly part: number;
  readonly total: number;
}

function sumRatio(
  rows: readonly HarnessMetrics[],
  part: (row: HarnessMetrics) => number | null,
  total: (row: HarnessMetrics) => number | null,
): Ratio | null {
  let known = false;
  let partSum = 0;
  let totalSum = 0;
  for (const row of rows) {
    const p = part(row);
    const t = total(row);
    if (p === null || t === null) continue;
    known = true;
    partSum += p;
    totalSum += t;
  }
  return known ? { part: partSum, total: totalSum } : null;
}

function rateText(ratio: Ratio | null): string {
  return ratio === null ? "—" : percent(ratio.part, ratio.total);
}

function perHundredText(ratio: Ratio | null): string {
  if (ratio === null || ratio.total === 0) return "—";
  return ((ratio.part / ratio.total) * 100).toFixed(1);
}

function perSessionText(ratio: Ratio | null): string {
  if (ratio === null || ratio.total === 0) return "—";
  return (ratio.part / ratio.total).toFixed(2);
}

export function durationText(seconds: number | null): string {
  if (seconds === null) return "—";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    const rest = seconds % 60;
    return rest === 0 ? `${minutes}m` : `${minutes}m ${rest}s`;
  }
  const rest = minutes % 60;
  return rest === 0
    ? `${Math.floor(minutes / 60)}h`
    : `${Math.floor(minutes / 60)}h ${rest}m`;
}

const toolErrors = (rows: readonly HarnessMetrics[]) =>
  sumRatio(
    rows,
    (row) => row.failedToolCalls,
    (row) => row.finishedToolCalls,
  );
const interrupts = (rows: readonly HarnessMetrics[]) =>
  sumRatio(
    rows,
    (row) => row.interrupts,
    (row) => (row.interrupts === null ? null : row.humanMessages),
  );
const corrections = (rows: readonly HarnessMetrics[]) =>
  sumRatio(
    rows,
    (row) => row.corrections,
    (row) => row.humanMessages,
  );
const cacheHits = (rows: readonly HarnessMetrics[]) =>
  sumRatio(
    rows,
    (row) => row.cacheReadTokens,
    (row) => row.cacheInputTokens,
  );
const compactions = (rows: readonly HarnessMetrics[]) =>
  sumRatio(
    rows,
    (row) => row.compactions,
    (row) => (row.compactions === null ? null : row.sessions),
  );

interface Headline {
  readonly title: string;
  readonly value: (metrics: SetupMetrics) => string;
}

const HEADLINES: readonly Headline[] = [
  {
    title: "Tool error rate",
    value: (metrics) => rateText(toolErrors(metrics.harnesses)),
  },
  {
    title: "Interrupts per 100 messages",
    value: (metrics) => perHundredText(interrupts(metrics.harnesses)),
  },
  {
    title: "Corrections per 100 messages",
    value: (metrics) => perHundredText(corrections(metrics.harnesses)),
  },
  {
    title: "Cache hit rate",
    value: (metrics) => rateText(cacheHits(metrics.harnesses)),
  },
  {
    title: "Compactions per session",
    value: (metrics) => perSessionText(compactions(metrics.harnesses)),
  },
];

export function MetricsPage({
  toolName,
}: {
  readonly toolName: string | null;
}) {
  const [filters, setFilters] = useToolCallFilters();
  return toolName === null ? (
    <MetricsOverview filters={filters} onFiltersChange={setFilters} />
  ) : (
    <ToolCallDetail
      key={toolName}
      toolName={toolName}
      filters={filters}
      onFiltersChange={setFilters}
    />
  );
}

function MetricsOverview({
  filters,
  onFiltersChange,
}: {
  readonly filters: ToolCallFilters;
  readonly onFiltersChange: (next: ToolCallFilters) => void;
}) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getSetupMetrics(filters),
    JSON.stringify(filters),
  );
  useEffect(
    () => commands.onHistoryImported(resource.reload),
    [commands, resource.reload],
  );

  return (
    <div
      className={`page ${resource.state === "loading" ? "is-refreshing" : ""}`}
    >
      <PageHeader title="Metrics" />
      <section className="toolbar toolbar--wrap" aria-label="Metric filters">
        <FilterBar filters={filters} onChange={onFiltersChange} />
      </section>
      {resource.state === "error" ? (
        <ErrorState
          title="Metrics are unavailable"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {resource.data === null ? (
        resource.state === "loading" ? (
          <LoadingState label="Measuring your setup" />
        ) : null
      ) : (
        <MetricsContent data={resource.data} filters={filters} />
      )}
    </div>
  );
}

function MetricsContent({
  data,
  filters,
}: {
  readonly data: SetupMetricsResponse;
  readonly filters: ToolCallFilters;
}) {
  const { current, previous } = data;
  if (current.harnesses.length === 0) {
    return (
      <>
        <RouteNotice meta={data.meta} />
        <EmptyState
          title="No activity"
          description="No captured sessions match these filters."
        />
      </>
    );
  }
  return (
    <>
      <RouteNotice meta={data.meta} />
      <section className="stat-grid metrics-grid" aria-label="Headline metrics">
        {HEADLINES.map((headline) => (
          <article className="stat-card" key={headline.title}>
            <div className="stat-card__topline">
              <span>{headline.title}</span>
            </div>
            <strong className="stat-card__value">
              {headline.value(current)}
            </strong>
            {previous === null ? null : (
              <p>Previous: {headline.value(previous)}</p>
            )}
          </article>
        ))}
        <article className="stat-card">
          <div className="stat-card__topline">
            <span>Autonomy span</span>
          </div>
          <ul className="metrics-autonomy" aria-label="Median autonomy span">
            {current.harnesses.map((row) => (
              <li key={row.harness}>
                <span>{HARNESS_NAMES[row.harness]}</span>
                <strong>{durationText(row.medianAutonomySeconds)}</strong>
              </li>
            ))}
          </ul>
        </article>
      </section>

      <div className="dashboard-grid">
        <HarnessTable
          rows={current.harnesses}
          previous={previous?.harnesses ?? null}
        />
        <ToolCallsPanel filters={filters} />
        <RepeatFailures metrics={current} />
        <McpServers metrics={current} />
        <Projects metrics={current} />
        <UnusedList
          id="unused-skills"
          title="Unused skills"
          names={data.unusedSkills}
        />
        <UnusedList
          id="unused-mcp"
          title="Unused MCP servers"
          names={data.unusedMcpServers}
        />
      </div>
    </>
  );
}

function HarnessTable({
  rows,
  previous,
}: {
  readonly rows: readonly HarnessMetrics[];
  readonly previous: readonly HarnessMetrics[] | null;
}) {
  const before = (harness: Harness) =>
    previous?.find((row) => row.harness === harness) ?? null;
  return (
    <section
      className="panel panel--span-2"
      aria-labelledby="harness-metrics-heading"
    >
      <div className="panel__header">
        <h2 id="harness-metrics-heading">By harness</h2>
      </div>
      <div className="tool-table-scroll">
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Harness</th>
              <th scope="col" className="numeric">
                Sessions
              </th>
              <th scope="col" className="numeric">
                Tool errors
              </th>
              <th scope="col" className="numeric">
                Interrupts /100
              </th>
              <th scope="col" className="numeric">
                Corrections /100
              </th>
              <th scope="col" className="numeric">
                Autonomy
              </th>
              <th scope="col" className="numeric">
                Cache hit
              </th>
              <th scope="col" className="numeric">
                Compactions /session
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const old = before(row.harness);
              const cell = (
                value: (rows: readonly HarnessMetrics[]) => string,
              ) => (
                <td className="numeric">
                  {value([row])}
                  {old === null ? null : (
                    <span className="metrics-previous">{value([old])}</span>
                  )}
                </td>
              );
              return (
                <tr key={row.harness}>
                  <th scope="row">
                    <HarnessLabel harness={row.harness} />
                  </th>
                  <td className="numeric">{formatInteger(row.sessions)}</td>
                  {cell((r) => rateText(toolErrors(r)))}
                  {cell((r) => perHundredText(interrupts(r)))}
                  {cell((r) => perHundredText(corrections(r)))}
                  {cell((r) =>
                    durationText(r[0]?.medianAutonomySeconds ?? null),
                  )}
                  {cell((r) => rateText(cacheHits(r)))}
                  {cell((r) => perSessionText(compactions(r)))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function RepeatFailures({ metrics }: { readonly metrics: SetupMetrics }) {
  return (
    <section className="panel" aria-labelledby="repeat-failures-heading">
      <div className="panel__header">
        <h2 id="repeat-failures-heading">Repeat failures</h2>
      </div>
      {metrics.repeatFailures.length === 0 ? (
        <EmptyState
          compact
          title="No repeat failures"
          description="No program failed three or more times."
        />
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Program</th>
              <th scope="col" className="numeric">
                Failures
              </th>
            </tr>
          </thead>
          <tbody>
            {metrics.repeatFailures.map((row) => (
              <tr key={row.program}>
                <th scope="row">
                  <a href={routeHref("/metrics", row.toolName)}>
                    {row.program}
                  </a>
                </th>
                <td className="numeric">{formatInteger(row.failures)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function McpServers({ metrics }: { readonly metrics: SetupMetrics }) {
  return (
    <section className="panel" aria-labelledby="mcp-servers-heading">
      <div className="panel__header">
        <h2 id="mcp-servers-heading">MCP servers</h2>
      </div>
      {metrics.mcpServers.length === 0 ? (
        <EmptyState
          compact
          title="No MCP calls"
          description="No MCP tool was called in this window."
        />
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Server</th>
              <th scope="col" className="numeric">
                Calls
              </th>
              <th scope="col" className="numeric">
                Error rate
              </th>
            </tr>
          </thead>
          <tbody>
            {metrics.mcpServers.map((row) => (
              <tr key={row.server}>
                <th scope="row">{row.server}</th>
                <td className="numeric">{formatInteger(row.calls)}</td>
                <td className="numeric">{percent(row.failed, row.calls)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function Projects({ metrics }: { readonly metrics: SetupMetrics }) {
  const showCost = metrics.projects.some(
    (row) => row.providerCostMicros !== null,
  );
  return (
    <section className="panel panel--span-2" aria-labelledby="projects-heading">
      <div className="panel__header">
        <h2 id="projects-heading">Projects</h2>
      </div>
      {metrics.projects.length === 0 ? (
        <EmptyState
          compact
          title="No token usage"
          description="No session reported usage in this window."
        />
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Project</th>
              <th scope="col" className="numeric">
                Sessions
              </th>
              <th scope="col" className="numeric">
                Input tokens
              </th>
              <th scope="col" className="numeric">
                Output tokens
              </th>
              {showCost ? (
                <th scope="col" className="numeric">
                  Reported cost
                </th>
              ) : null}
            </tr>
          </thead>
          <tbody>
            {metrics.projects.map((row) => (
              <tr key={row.project ?? ""}>
                <th scope="row">{row.project ?? "No project"}</th>
                <td className="numeric">{formatInteger(row.sessions)}</td>
                <td className="numeric">{formatCompact(row.inputTokens)}</td>
                <td className="numeric">{formatCompact(row.outputTokens)}</td>
                {showCost ? (
                  <td className="numeric">
                    {row.providerCostMicros === null
                      ? "—"
                      : formatMicros(row.providerCostMicros)}
                  </td>
                ) : null}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function UnusedList({
  id,
  title,
  names,
}: {
  readonly id: string;
  readonly title: string;
  readonly names: readonly string[] | null;
}) {
  return (
    <section className="panel" aria-labelledby={`${id}-heading`}>
      <div className="panel__header">
        <h2 id={`${id}-heading`}>
          {title}
          {names === null ? null : ` · ${formatInteger(names.length)}`}
        </h2>
      </div>
      {names === null ? (
        <EmptyState
          compact
          title="Inventory unavailable"
          description="Installed items could not be read."
        />
      ) : names.length === 0 ? (
        <EmptyState
          compact
          title="Everything was used"
          description="Every installed item was called in this window."
        />
      ) : (
        <ul className="metrics-names">
          {names.map((name) => (
            <li key={name}>{name}</li>
          ))}
        </ul>
      )}
    </section>
  );
}

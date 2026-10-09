import { ArrowRight, CircleDashed, ShieldAlert } from "lucide-react";
import { useEffect, useMemo } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { DashboardResponse } from "../contracts.js";
import {
  estimateCost,
  formatCompact,
  formatInteger,
  formatMicros,
  quotaPercent,
  reconcileUsage,
  usageByHarness,
} from "../domain/reconcile.js";
import { routeHref } from "../router.js";
import {
  EmptyState,
  ErrorState,
  LoadingState,
  PageHeader,
} from "../components/Primitives.js";
import { HARNESS_NAMES, HarnessLabel } from "../components/HarnessMark.js";
import { RouteNotice } from "../components/Notices.js";

export function DashboardPage() {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getDashboard(),
    "dashboard",
  );
  // Sessions created since the page opened arrive through background imports.
  useEffect(
    () => commands.onHistoryImported(resource.reload),
    [commands, resource.reload],
  );

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Loading usage" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Dashboard data is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data;
  if (data === null) return null;

  return (
    <DashboardContent data={data} refreshing={resource.state === "loading"} />
  );
}

function DashboardContent({
  data,
  refreshing,
}: {
  readonly data: DashboardResponse;
  readonly refreshing: boolean;
}) {
  const usage = useMemo(() => reconcileUsage(data.sessions), [data.sessions]);
  const harnessUsage = useMemo(
    () => usageByHarness(data.sessions),
    [data.sessions],
  );
  const estimatedCost = useMemo(
    () => estimateCost(data.sessions, data.prices),
    [data.prices, data.sessions],
  );
  const activeHarnessUsage = harnessUsage.filter((row) => row.sessionCount > 0);
  const knownGrandTotal = activeHarnessUsage.reduce(
    (total, row) => total + (row.totalTokens ?? 0),
    0,
  );
  const grandTotal = activeHarnessUsage.every((row) => row.totalTokens !== null)
    ? knownGrandTotal
    : null;

  return (
    <div className={`page ${refreshing ? "is-refreshing" : ""}`}>
      <PageHeader
        title="Overview"
        actions={
          <a
            className="button button--secondary button--regular"
            href={routeHref("/sessions")}
          >
            Browse sessions <ArrowRight aria-hidden="true" />
          </a>
        }
      />
      <RouteNotice meta={data.meta} />

      {data.sessions.length === 0 ? (
        <EmptyState
          title="No sessions yet"
          description="Usage appears after your first session."
          action={
            <a
              className="button button--primary button--regular"
              href={routeHref("/onboarding")}
            >
              Set up capture
            </a>
          }
        />
      ) : (
        <>
          <section className="stat-grid" aria-label="Totals">
            <article className="stat-card">
              <div className="stat-card__topline">
                <span>Sessions in view</span>
                {data.captureLive ? (
                  <span className="live-indicator">
                    <span aria-hidden="true" /> Live capture
                  </span>
                ) : (
                  <span className="quiet-indicator">Capture idle</span>
                )}
              </div>
              <strong className="stat-card__value">{usage.sessions}</strong>
            </article>
            <article className="stat-card">
              <div className="stat-card__topline">
                <span>Input tokens</span>
              </div>
              <strong className="stat-card__value">
                {formatCompact(usage.inputTokens.value)}
              </strong>
            </article>
            <article className="stat-card">
              <div className="stat-card__topline">
                <span>Output tokens</span>
              </div>
              <strong className="stat-card__value">
                {formatCompact(usage.outputTokens.value)}
              </strong>
            </article>
            <article className="stat-card">
              <div className="stat-card__topline">
                <span>Cost in view</span>
              </div>
              <strong className="stat-card__value">
                {formatMicros(estimatedCost.micros)}
              </strong>
              <p>
                {estimatedCost.label === "estimated" ? "Estimated" : "Unknown"}
              </p>
            </article>
          </section>

          <div className="dashboard-grid">
            <section
              className="panel panel--span-2"
              aria-labelledby="usage-heading"
            >
              <div className="panel__header">
                <div>
                  <h2 id="usage-heading">Token usage</h2>
                </div>
              </div>
              <div
                className="chart-legend"
                role="list"
                aria-label="Harness legend"
              >
                {harnessUsage.map((row) => (
                  <span role="listitem" key={row.harness}>
                    <span
                      className={`legend-mark legend-mark--${row.harness}`}
                      aria-hidden="true"
                    />
                    <HarnessLabel harness={row.harness} />
                  </span>
                ))}
              </div>
              <div
                className="usage-chart"
                role="group"
                aria-label="Token usage plot by harness"
              >
                {harnessUsage.map((row) => {
                  if (row.totalTokens === null) return null;
                  const width =
                    knownGrandTotal === 0
                      ? 0
                      : (row.totalTokens / knownGrandTotal) * 100;
                  return (
                    <span
                      key={row.harness}
                      className={`usage-chart__segment usage-chart__segment--${row.harness}`}
                      style={{ width: `${width}%` }}
                      role="img"
                      tabIndex={0}
                      aria-label={`${HARNESS_NAMES[row.harness]}: ${formatInteger(row.totalTokens)} tokens`}
                      data-tooltip={`${HARNESS_NAMES[row.harness]}: ${formatInteger(row.totalTokens)} tokens`}
                    />
                  );
                })}
              </div>
              {grandTotal === null ? (
                <p className="chart-note">
                  Partial chart: harnesses with unknown input or output totals
                  are omitted, not drawn as zero.
                </p>
              ) : null}
              <table
                className="data-table data-table--compact"
                aria-label="Raw values used by the token usage chart"
              >
                <thead>
                  <tr>
                    <th scope="col">Harness</th>
                    <th scope="col" className="numeric">
                      Sessions
                    </th>
                    <th scope="col" className="numeric">
                      Input
                    </th>
                    <th scope="col" className="numeric">
                      Output
                    </th>
                    <th scope="col" className="numeric">
                      Total
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {harnessUsage.map((row) => (
                    <tr key={row.harness}>
                      <th scope="row">
                        <span
                          className={`legend-mark legend-mark--${row.harness}`}
                          aria-hidden="true"
                        />
                        <HarnessLabel harness={row.harness} />
                      </th>
                      <td className="numeric">{row.sessionCount}</td>
                      <td className="numeric">
                        {formatInteger(row.inputTokens)}
                      </td>
                      <td className="numeric">
                        {formatInteger(row.outputTokens)}
                      </td>
                      <td className="numeric">
                        <strong>{formatInteger(row.totalTokens)}</strong>
                      </td>
                    </tr>
                  ))}
                </tbody>
                <tfoot>
                  <tr>
                    <th scope="row">Total</th>
                    <td className="numeric">{usage.sessions}</td>
                    <td className="numeric">
                      {formatInteger(usage.inputTokens.value)}
                    </td>
                    <td className="numeric">
                      {formatInteger(usage.outputTokens.value)}
                    </td>
                    <td className="numeric">
                      <strong>{formatInteger(grandTotal)}</strong>
                    </td>
                  </tr>
                </tfoot>
              </table>
            </section>

            {data.quotas.length === 0 ? null : (
              <section className="panel" aria-labelledby="quota-heading">
                <div className="panel__header">
                  <div>
                    <h2 id="quota-heading">Quota windows</h2>
                  </div>
                </div>
                <div className="quota-list">
                  {data.quotas.map((quota) => {
                    const percent = quotaPercent(quota);
                    return (
                      <article className="quota-item" key={quota.name}>
                        <div className="quota-item__title">
                          <strong>{quota.name}</strong>
                        </div>
                        {percent === null ? (
                          <div className="unknown-meter">
                            <CircleDashed aria-hidden="true" />
                            <span>Unknown</span>
                          </div>
                        ) : (
                          <div>
                            <div
                              className="meter"
                              role="meter"
                              aria-label={`${quota.name} remaining`}
                              aria-valuemin={0}
                              aria-valuemax={quota.limit ?? undefined}
                              aria-valuenow={quota.remaining ?? undefined}
                              aria-valuetext={`${quota.remaining} of ${quota.limit} remaining, estimated`}
                            >
                              <span style={{ width: `${percent}%` }} />
                            </div>
                            <p>
                              <strong>{quota.remaining}</strong> of{" "}
                              {quota.limit} remaining
                            </p>
                          </div>
                        )}
                      </article>
                    );
                  })}
                </div>
              </section>
            )}

            {data.conflicts.length === 0 ? null : (
              <section
                className="panel panel--span-2"
                aria-labelledby="reconcile-heading"
              >
                <div className="panel__header">
                  <div>
                    <h2 id="reconcile-heading">Overlapping sources</h2>
                  </div>
                </div>
                <div className="conflict-list">
                  {data.conflicts.map((conflict) => (
                    <article key={conflict.fact}>
                      <ShieldAlert aria-hidden="true" />
                      <div>
                        <strong>{conflict.fact}</strong>
                        <p>{conflict.reason}</p>
                        <dl>
                          <div>
                            <dt>Counted</dt>
                            <dd>{conflict.winner}</dd>
                          </div>
                          <div>
                            <dt>Retained, not added</dt>
                            <dd>{conflict.ignored}</dd>
                          </div>
                        </dl>
                      </div>
                    </article>
                  ))}
                </div>
              </section>
            )}
          </div>
        </>
      )}
    </div>
  );
}

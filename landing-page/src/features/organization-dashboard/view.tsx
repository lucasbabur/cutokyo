"use client";

import Link from "next/link";
import { useMemo, useState } from "react";

import { tokenSavingEnabled } from "@/shared/config/features";
import { AdminShell } from "@/shared/ui/admin-shell";

const MANAGED_CONTROLS_COPY = tokenSavingEnabled
  ? "managed redaction or compression."
  : "managed redaction.";

import {
  openOrganizationIdentitySetup,
  useOrganizationDashboard,
  useOrganizationOnboarding,
} from "./model";
import styles from "./view.module.css";

import type {
  OrganizationOperationPage,
  OrganizationSessionDetail,
  OrganizationTelemetry,
  PolicyCapability,
  ReportFilters,
  TelemetryDimension,
  TelemetryScope,
} from "./model";
import type { FormEvent } from "react";

type OrganizationDashboardSection =
  | "activity"
  | "employees"
  | "enterprise"
  | "governance"
  | "overview"
  | "sessions";

const SECTION_COPY: Record<OrganizationDashboardSection, { eyebrow: string; title: string }> = {
  activity: { eyebrow: "Endpoint operations", title: "Activity ledger" },
  employees: { eyebrow: "Organization directory", title: "Employee usage" },
  enterprise: { eyebrow: "Organization defaults", title: "Enterprise controls" },
  governance: { eyebrow: "Policy & security", title: "Governance controls" },
  overview: { eyebrow: "Organization control plane", title: "AI endpoint observability" },
  sessions: { eyebrow: "Redacted organization history", title: "Employee sessions" },
};

export function OrganizationDashboardView({
  section = "overview",
  workosConfigured = true,
}: {
  section?: OrganizationDashboardSection;
  workosConfigured?: boolean;
}) {
  if (!workosConfigured) {
    return (
      <main className={styles.centered}>
        <span className={styles.wordmark}>cutokyo / admin</span>
        <h1>Organization identity is not configured.</h1>
        <p>Add the identity service credentials to enable the enterprise control plane.</p>
      </main>
    );
  }
  return <ConfiguredOrganizationDashboardView section={section} />;
}

// eslint-disable-next-line local/no-complex-business-logic -- Selects render-only authentication and API states from the dashboard model.
function ConfiguredOrganizationDashboardView({
  section,
}: {
  section: OrganizationDashboardSection;
}) {
  const returnTo = section === "overview" ? "/admin" : `/admin/${section}`;
  const {
    data,
    deleteGroup,
    error,
    exportOrganizationData,
    inviteMember,
    invitingMember,
    loadOperations,
    loading,
    operationCorrelation,
    operationError,
    operationLoading,
    operationPage,
    organizationId,
    permissions,
    reportFilters,
    saveGroup,
    saveManagedPlugin,
    savingGroup,
    savingPolicy,
    savingPlugin,
    selectedSessionId,
    sessionDetail,
    sessionError,
    sessionLoading,
    setSelectedSessionId,
    setPolicyCapability,
    setReportFilters,
    setTelemetryScope,
    switchToOrganization,
    telemetryScope,
    testManagedPlugin,
    uninstallManagedPlugin,
    user,
  } = useOrganizationDashboard();

  if (!user) {
    return (
      <main className={styles.centered}>
        <span className={styles.wordmark}>cutokyo / admin</span>
        <h1>Organization observability starts here.</h1>
        <p>Sign in to access your organization’s governed AI traffic.</p>
        <a href={`/sign-in?returnTo=${encodeURIComponent(returnTo)}`}>
          Sign in to your organization
        </a>
      </main>
    );
  }

  if (!organizationId) {
    return <OrganizationOnboarding onCreated={switchToOrganization} />;
  }

  return (
    <AdminShell
      active={section}
      organizationId={organizationId}
      organizationName={data?.organization?.name}
      user={user}
    >
      <section className={styles.workspace}>
        <header>
          <div>
            <p>{SECTION_COPY[section].eyebrow}</p>
            <h1>{SECTION_COPY[section].title}</h1>
          </div>
          <div className={styles.headerActions}>
            <Link className={styles.accountButton} href="/admin/account">
              Account
            </Link>
            <div className={styles.liveBadge}>
              <i /> Governed telemetry
            </div>
          </div>
        </header>

        {data && data.license.mode !== "disabled" && data.license.state !== "active" ? (
          <LicenseBanner license={data.license} onExport={exportOrganizationData} />
        ) : null}

        {loading && !data ? (
          <DashboardSkeleton section={section} />
        ) : (
          <>
            {error ? (
              <div className={styles.error} role="alert">
                {error}
              </div>
            ) : null}
            {loading ? (
              <div className={styles.loading} role="status">
                Loading organization telemetry…
              </div>
            ) : null}

            {section !== "governance" && section !== "enterprise" ? (
              <TelemetryScopePicker
                groups={data?.groups ?? []}
                members={data?.members ?? []}
                onChange={setTelemetryScope}
                scope={telemetryScope}
                users={data?.byUser ?? []}
              />
            ) : null}

            {section === "overview" ? (
              <>
                <ReportToolbar data={data} filters={reportFilters} onChange={setReportFilters} />
                <section className={styles.metrics}>
                  <Metric label="Provider tokens" value={compact(data?.totalTokens ?? 0)} />
                  <Metric label="Estimated cost" value={usd(data?.totalCostNanosUsd ?? 0)} />
                  <Metric label="Cached input" value={compact(data?.cacheReadTokens ?? 0)} />
                  {tokenSavingEnabled ? (
                    <Metric
                      accent
                      label="Tokens removed"
                      value={compact(data?.estimatedTokensSaved ?? 0)}
                    />
                  ) : null}
                  <Metric label="Operations" value={compact(data?.totalEvents ?? 0)} />
                  <Metric label="Errors" value={compact(data?.errorEvents ?? 0)} />
                </section>
                <ReportCharts timeline={data?.timeline ?? []} />
                <section className={styles.grid}>
                  <Breakdown dimensions={data?.byHarness ?? []} title="Traffic by harness" />
                  <Breakdown dimensions={data?.byProvider ?? []} title="Providers" />
                  <Breakdown dimensions={data?.byModel ?? []} title="Models" />
                  <Breakdown
                    dimensions={data?.byUser ?? []}
                    label={(item) => memberEmail(data?.members ?? [], item.name)}
                    title="Users"
                  />
                  <Breakdown dimensions={data?.byGroup ?? []} title="Groups" />
                </section>
                <ContextPanel context={data?.context} />
              </>
            ) : null}

            {section === "employees" ? (
              <>
                <EmployeePanel
                  members={data?.members ?? []}
                  sessions={data?.sessions ?? []}
                  usage={data?.byUser ?? []}
                />
                <InviteEmployeePanel
                  canInvite={permissions.some((permission) =>
                    ["groups:write", "members:write", "widgets:users-table:manage"].includes(
                      permission,
                    ),
                  )}
                  disabled={invitingMember || data?.license.readOnly === true}
                  invitations={data?.invitations ?? []}
                  onInvite={inviteMember}
                />
                <GroupPanel
                  canWrite={permissions.includes("groups:write") && data?.license.readOnly !== true}
                  disabled={savingGroup || data?.license.readOnly === true}
                  groups={data?.groups ?? []}
                  onDelete={(groupId) => deleteGroup(groupId)}
                  onSave={(input) => saveGroup(input)}
                  members={data?.members ?? []}
                />
              </>
            ) : null}

            {section === "governance" ? (
              <section className={styles.adminGrid}>
                <DevicePanel devices={data?.devices ?? []} />
                <AuditPanel events={data?.auditEvents ?? []} />
              </section>
            ) : null}

            {section === "enterprise" ? (
              <EnterprisePolicyPanel
                canWrite={permissions.includes("policies:write") && data?.license.readOnly !== true}
                disabled={savingPolicy || data?.license.readOnly === true}
                onChange={(capability, enabled) => void setPolicyCapability(capability, enabled)}
                permissions={permissions}
                plugins={data?.plugins ?? []}
                policy={data?.policy ?? null}
                pluginDisabled={savingPlugin || data?.license.readOnly === true}
                onPluginSave={saveManagedPlugin}
                onPluginTest={testManagedPlugin}
                onPluginUninstall={uninstallManagedPlugin}
              />
            ) : null}

            {section === "sessions" ? (
              <SessionPanel
                canRead={permissions.includes("sessions:read")}
                detail={sessionDetail}
                error={sessionError}
                loading={sessionLoading}
                members={data?.members ?? []}
                onSelect={setSelectedSessionId}
                selectedId={selectedSessionId}
                sessions={data?.sessions ?? []}
              />
            ) : null}

            {section === "activity" ? (
              <OperationInvestigationPanel
                canRead={permissions.includes("telemetry:read")}
                correlation={operationCorrelation}
                error={operationError}
                loading={operationLoading}
                members={data?.members ?? []}
                onLoad={loadOperations}
                page={
                  operationPage ?? {
                    nextCursor: null,
                    operations: data?.recentEvents ?? [],
                  }
                }
              />
            ) : null}

            {section === "governance" ? (
              <section className={styles.governance}>
                <article>
                  <span>Tenant boundary</span>
                  <strong>Organization identity JWT</strong>
                  <p>Every query is scoped to the active organization and permission claims.</p>
                </article>
                <article>
                  <span>Content handling</span>
                  <strong>{compact(data?.totalRedactions ?? 0)} values redacted</strong>
                  <p>
                    Raw capture stays off; policy-governed session previews contain redacted text
                    only.
                  </p>
                </article>
                <article>
                  <span>Endpoint fleet</span>
                  <strong>{data?.byDevice.length ?? 0} active sources</strong>
                  <p>
                    Credentials are short-lived, revocable, and stored only on enrolled devices.
                  </p>
                </article>
                <article>
                  <span>Context telemetry</span>
                  <strong>{compact(data?.totalTokens ?? 0)} observed tokens</strong>
                  <p>Token attribution and redacted previews stay scoped to this organization.</p>
                </article>
              </section>
            ) : null}
          </>
        )}
      </section>
    </AdminShell>
  );
}

type OrganizationOperation = OrganizationOperationPage["operations"][number];

// eslint-disable-next-line local/no-complex-business-logic -- Renders explicit permission, search, empty, error, and cursor states supplied by the dashboard model.
function OperationInvestigationPanel({
  canRead,
  correlation,
  error,
  loading,
  members,
  onLoad,
  page,
}: {
  canRead: boolean;
  correlation: string;
  error: string | null;
  loading: boolean;
  members: OrganizationTelemetry["members"];
  onLoad: (correlation: string, cursor?: number) => Promise<void>;
  page: OrganizationOperationPage;
}) {
  const [query, setQuery] = useState(correlation);
  return (
    <section className={styles.activity}>
      <div className={styles.sectionTitle}>
        <div>
          <span>{correlation ? "Exact correlation" : "Policy-safe telemetry"}</span>
          <h2>Operation investigation</h2>
        </div>
        <em>{page.operations.length} shown</em>
      </div>
      {!canRead ? (
        <p className={styles.empty}>Your role does not grant telemetry:read.</p>
      ) : (
        <>
          <form
            className={styles.operationSearch}
            onSubmit={(event) => {
              event.preventDefault();
              void onLoad(query);
            }}
          >
            <label>
              <span>Exact trace, request, or provider response ID</span>
              <input
                aria-label="Trace, request, or provider response ID"
                maxLength={240}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="trace_…, request_…, or response_…"
                type="search"
                value={query}
              />
            </label>
            <button disabled={loading} type="submit">
              {loading ? "Searching…" : "Find exact"}
            </button>
            {correlation ? (
              <button
                disabled={loading}
                onClick={() => {
                  setQuery("");
                  void onLoad("");
                }}
                type="button"
              >
                Reset
              </button>
            ) : null}
          </form>
          {error ? (
            <p className={styles.error} role="alert">
              {error}
            </p>
          ) : null}
          <div className={styles.operationList}>
            {page.operations.map((operation) => (
              <OrganizationOperationCard
                key={`${operation.deviceId}-${operation.traceId}`}
                members={members}
                operation={operation}
              />
            ))}
          </div>
          {!page.operations.length && !loading ? (
            <p className={styles.empty}>
              {correlation
                ? "No operation has that exact tenant-scoped identifier."
                : "No organization events yet. Install the desktop, sign in, and run an AI tool."}
            </p>
          ) : null}
          {page.nextCursor ? (
            <button
              className={styles.loadOlderButton}
              disabled={loading}
              onClick={() => void onLoad(correlation, page.nextCursor ?? undefined)}
              type="button"
            >
              Load older operations
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}

function OrganizationOperationCard({
  members,
  operation,
}: {
  members: OrganizationTelemetry["members"];
  operation: OrganizationOperation;
}) {
  const failure = operation.errorType ?? operation.outcome;
  return (
    <details className={styles.operationCard}>
      <summary>
        <span>
          <strong>{operation.requestId ?? operation.traceId}</strong>
          <small>
            {displayName(operation.provider)} · {operation.model ?? "model unavailable"}
          </small>
        </span>
        <span>
          <strong>{compact(operation.totalTokens)} tokens</strong>
          <small>
            {time(operation.timestamp)} · {operation.durationMs} ms
          </small>
        </span>
        <span className={operation.outcome === "success" ? styles.ok : styles.bad}>
          {operation.statusCode} · {failure}
        </span>
      </summary>
      <dl className={styles.operationDetails}>
        <OperationField label="Trace ID" value={operation.traceId} />
        <OperationField label="Request ID" value={operation.requestId} />
        <OperationField label="Provider response ID" value={operation.providerResponseId} />
        <OperationField label="Conversation ID" value={operation.conversationId} />
        <OperationField label="Outcome / failure" value={`${operation.outcome} · ${failure}`} />
        <OperationField
          label="API surface / transport"
          value={`${operation.apiSurface} · ${operation.transport}`}
        />
        <OperationField
          label="Harness attribution"
          value={`${displayName(operation.harness)} · ${operation.harnessMethod} · ${Math.round(operation.harnessConfidence * 100)}%`}
        />
        <OperationField label="Employee" value={memberEmail(members, operation.userId)} />
        <OperationField
          label="Timing (total / local / provider / TTFB)"
          value={`${operation.durationMs} / ${optionalMs(operation.localProcessingMs)} / ${optionalMs(operation.providerDurationMs)} / ${optionalMs(operation.timeToFirstByteMs)}`}
        />
        <OperationField label="Usage source" value={operation.usageSource} />
        <OperationField
          label="Tokens (input / total input / output)"
          value={`${operation.inputTokens} / ${operation.totalInputTokens} / ${operation.outputTokens}`}
        />
        <OperationField
          label="Tokens (cache read / write / reasoning / total)"
          value={`${operation.cacheReadTokens} / ${operation.cacheWriteTokens} / ${operation.reasoningTokens} / ${operation.totalTokens}`}
        />
        <OperationField
          label="Cost / pricing version"
          value={`${usd(operation.costNanosUsd)} · ${operation.pricingVersion ?? "unavailable"}`}
        />
        <OperationField
          label="Optimization"
          value={`${operation.estimatedTokensSaved} saved · ${operation.optimizationChangedFields} fields · ${jsonSummary(operation.optimization)}`}
        />
        <OperationField
          label="Security"
          value={`${operation.redactions} redactions · ${jsonSummary(operation.security)}`}
        />
        <OperationField label="Plugins" value={jsonSummary(operation.plugins)} />
        <OperationField label="Preview state" value={previewLabel(operation.previewState)} />
      </dl>
    </details>
  );
}

function OperationField({
  label,
  value,
}: {
  label: string;
  value: number | string | null | undefined;
}) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value ?? "Unavailable"}</dd>
    </div>
  );
}

function optionalMs(value: number | null | undefined): string {
  return value == null ? "—" : `${value} ms`;
}

function jsonSummary(value: object | null | undefined): string {
  return JSON.stringify(value ?? {});
}

function previewLabel(state: OrganizationOperation["previewState"]): string {
  return state === "redacted_preview" ? "Policy-redacted preview available" : "Metadata only";
}

function LicenseBanner({
  license,
  onExport,
}: {
  license: OrganizationTelemetry["license"];
  onExport: () => Promise<void>;
}) {
  const grace = license.graceUntil ? new Date(license.graceUntil).toLocaleString() : null;
  const readOnly = license.readOnly;
  return (
    <aside className={readOnly ? styles.licenseBlocked : styles.licenseWarning} role="status">
      <div>
        <strong>
          {readOnly ? "This installation is read-only" : "Offline license grace is active"}
        </strong>
        <p>{license.detail}</p>
      </div>
      <div>
        <span>{license.state}</span>
        {grace ? <small>Grace deadline: {grace}</small> : null}
        <small>Reads, export, backups, and local safety remain available.</small>
        <button onClick={() => void onExport()} type="button">
          Export organization data
        </button>
      </div>
    </aside>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Chooses a fixed geometry for each dashboard route while its data is loading.
function DashboardSkeleton({ section }: { section: OrganizationDashboardSection }) {
  return (
    <section
      aria-label="Loading organization telemetry"
      aria-live="polite"
      className={styles.skeleton}
      data-testid="dashboard-skeleton"
      role="status"
    >
      <span className={styles.skeletonStatus}>Loading organization telemetry…</span>
      {section !== "enterprise" && section !== "governance" ? <SkeletonBlock kind="scope" /> : null}
      {section === "overview" ? (
        <>
          <SkeletonBlock kind="toolbar" />
          <div className={styles.skeletonMetrics}>
            {Array.from({ length: 6 }, (_, index) => (
              <SkeletonBlock key={index} kind="metric" />
            ))}
          </div>
          <SkeletonBlock kind="chart" />
          <div className={styles.skeletonGrid}>
            {Array.from({ length: 5 }, (_, index) => (
              <SkeletonBlock key={index} kind="card" />
            ))}
          </div>
        </>
      ) : null}
      {section === "employees" ? (
        <div className={styles.skeletonStack}>
          <SkeletonBlock kind="table" />
          <SkeletonBlock kind="form" />
          <SkeletonBlock kind="form" />
        </div>
      ) : null}
      {section === "sessions" ? (
        <>
          <SkeletonBlock kind="toolbar" />
          <div className={styles.skeletonSplit}>
            <SkeletonBlock kind="table" />
            <SkeletonBlock kind="detail" />
          </div>
        </>
      ) : null}
      {section === "activity" ? <SkeletonBlock kind="table" /> : null}
      {section === "governance" ? (
        <>
          <div className={styles.skeletonSplit}>
            <SkeletonBlock kind="detail" />
            <SkeletonBlock kind="detail" />
          </div>
          <div className={styles.skeletonMetrics}>
            {Array.from({ length: 4 }, (_, index) => (
              <SkeletonBlock key={index} kind="metric" />
            ))}
          </div>
        </>
      ) : null}
      {section === "enterprise" ? (
        <div className={styles.skeletonStack}>
          <SkeletonBlock kind="detail" />
          <SkeletonBlock kind="form" />
          <SkeletonBlock kind="form" />
        </div>
      ) : null}
    </section>
  );
}

function SkeletonBlock({
  kind,
}: {
  kind: "card" | "chart" | "detail" | "form" | "metric" | "scope" | "table" | "toolbar";
}) {
  return (
    <span aria-hidden="true" className={`${styles.skeletonBlock} ${styles[`skeleton_${kind}`]}`} />
  );
}

function ReportToolbar({
  data,
  filters,
  onChange,
}: {
  data: OrganizationTelemetry | null;
  filters: ReportFilters;
  onChange: (filters: ReportFilters) => void;
}) {
  const providers = reportOptions(data?.byProvider ?? [], filters.provider);
  const models = reportOptions(data?.byModel ?? [], filters.model);
  const harnesses = reportOptions(data?.byHarness ?? [], filters.harness);
  return (
    <section className={styles.reportToolbar}>
      <div>
        <span>Report window</span>
        <select
          aria-label="Report time range"
          onChange={(event) =>
            onChange({ ...filters, range: event.target.value as ReportFilters["range"] })
          }
          value={filters.range}
        >
          <option value="24h">Last 24 hours</option>
          <option value="7d">Last 7 days</option>
          <option value="30d">Last 30 days</option>
          <option value="90d">Last 90 days</option>
          <option value="all">All time</option>
        </select>
      </div>
      <ReportSelect
        label="Provider"
        onChange={(provider) => onChange({ ...filters, provider })}
        options={providers}
        value={filters.provider}
      />
      <ReportSelect
        label="Model"
        onChange={(model) => onChange({ ...filters, model })}
        options={models}
        value={filters.model}
      />
      <ReportSelect
        label="Harness"
        onChange={(harness) => onChange({ ...filters, harness })}
        options={harnesses}
        value={filters.harness}
      />
      <div className={styles.reportActions}>
        <button
          disabled={!data?.recentEvents.length}
          onClick={() => exportTelemetryCsv(data?.recentEvents ?? [], data?.members ?? [])}
          type="button"
        >
          Export CSV
        </button>
        <button
          onClick={() => onChange({ harness: "all", model: "all", provider: "all", range: "30d" })}
          type="button"
        >
          Reset
        </button>
      </div>
    </section>
  );
}

function ReportSelect({
  label,
  onChange,
  options,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  options: string[];
  value: string;
}) {
  return (
    <label>
      <span>{label}</span>
      <select
        aria-label={`Report ${label}`}
        onChange={(event) => onChange(event.target.value)}
        value={value}
      >
        <option value="all">All {label.toLowerCase()}s</option>
        {options.map((option) => (
          <option key={option} value={option}>
            {displayName(option)}
          </option>
        ))}
      </select>
    </label>
  );
}

function ReportCharts({ timeline }: { timeline: OrganizationTelemetry["timeline"] }) {
  return (
    <section className={styles.reportCharts}>
      <TimelineChart
        accent="lime"
        label="Token volume"
        timeline={timeline}
        value={(bucket) => bucket.tokens}
        valueLabel={(value) => compact(value)}
      />
      <TimelineChart
        accent="blue"
        label="Estimated spend"
        timeline={timeline}
        value={(bucket) => bucket.costNanosUsd}
        valueLabel={usd}
      />
      <TimelineChart
        accent="red"
        label="Errors"
        timeline={timeline}
        value={(bucket) => bucket.errors}
        valueLabel={(value) => compact(value)}
      />
    </section>
  );
}

function TimelineChart({
  accent,
  label,
  timeline,
  value,
  valueLabel,
}: {
  accent: "blue" | "lime" | "red";
  label: string;
  timeline: OrganizationTelemetry["timeline"];
  value: (bucket: OrganizationTelemetry["timeline"][number]) => number;
  valueLabel: (value: number) => string;
}) {
  const values = timeline.map(value);
  const maximum = Math.max(...values, 1);
  const total = values.reduce((sum, item) => sum + item, 0);
  return (
    <article className={styles.timelineChart} data-accent={accent}>
      <header>
        <span>{label}</span>
        <strong>{valueLabel(total)}</strong>
      </header>
      <div aria-label={`${label} over time`} className={styles.chartBars} role="img">
        {timeline.map((bucket) => (
          <i
            key={bucket.timestamp}
            style={{ height: `${Math.max(3, (value(bucket) / maximum) * 100)}%` }}
            title={`${time(bucket.timestamp)}: ${valueLabel(value(bucket))}`}
          />
        ))}
        {!timeline.length ? <small>Waiting for report data</small> : null}
      </div>
      <footer>
        <span>{timeline[0] ? shortDate(timeline[0].timestamp) : "—"}</span>
        <span>{timeline.at(-1) ? shortDate(timeline.at(-1)?.timestamp ?? "") : "—"}</span>
      </footer>
    </article>
  );
}

function OrganizationOnboarding({
  onCreated,
}: {
  onCreated: (organizationId: string) => Promise<unknown>;
}) {
  const { error, pending, submit } = useOrganizationOnboarding(onCreated);
  return (
    <main className={styles.onboarding}>
      <span className={styles.wordmark}>cutokyo / enterprise</span>
      <p>Organization setup</p>
      <h1>Name your organization.</h1>
      <span>Create the workspace your employees, policies, sessions, and reports belong to.</span>
      <form onSubmit={(event) => void submit(event)}>
        <label>
          Organization name
          <input autoComplete="organization" maxLength={80} name="name" required />
        </label>
        <label>
          Company domain <small>optional</small>
          <input autoComplete="url" name="domain" placeholder="example.com" />
        </label>
        {error ? <strong role="alert">{error}</strong> : null}
        <button disabled={pending} type="submit">
          {pending ? "Creating organization…" : "Create organization"}
        </button>
      </form>
    </main>
  );
}
/**
 * Renders nothing in deployments that ship without token saving, so the
 * enterprise section carries no branch of its own for the feature.
 */
function TokenCompressionPanel({
  canWrite,
  disabled,
  onChange,
  policy,
}: {
  canWrite: boolean;
  disabled: boolean;
  onChange: (capability: PolicyCapability, enabled: boolean) => void;
  policy: OrganizationTelemetry["policy"];
}) {
  if (!tokenSavingEnabled) return null;
  const enabled = policy?.policy.optimization.enabled ?? false;
  return (
    <article className={styles.controlPanel}>
      <div className={styles.sectionTitle}>
        <div>
          <span>Optimization default</span>
          <h2>Token compression</h2>
        </div>
        <em>{enabled ? "Default on" : "Default off"}</em>
      </div>
      <div className={styles.featureControl}>
        <div>
          <strong>Compress eligible tool results</strong>
          <p>Apply the organization default before requests reach the AI provider.</p>
        </div>
        <button
          aria-label="Organization token compression default"
          aria-pressed={enabled}
          disabled={disabled || !canWrite || !policy}
          onClick={() => onChange("optimization", !enabled)}
          type="button"
        >
          <i />
        </button>
      </div>
    </article>
  );
}

function EnterprisePolicyPanel({
  canWrite,
  disabled,
  onChange,
  onPluginSave,
  onPluginTest,
  onPluginUninstall,
  permissions,
  pluginDisabled,
  plugins,
  policy,
}: {
  canWrite: boolean;
  disabled: boolean;
  onChange: (capability: PolicyCapability, enabled: boolean) => void;
  onPluginSave: (input: {
    enabled: boolean;
    endpoint: string;
    id?: string;
    name: string;
    replaceSecret?: boolean;
    secret?: string;
  }) => Promise<unknown>;
  onPluginTest: (pluginId: string) => Promise<unknown>;
  onPluginUninstall: (pluginId: string) => Promise<unknown>;
  permissions: string[];
  pluginDisabled: boolean;
  plugins: OrganizationTelemetry["plugins"];
  policy: OrganizationTelemetry["policy"];
}) {
  const redactionControls: [PolicyCapability, string, string, boolean][] = policy
    ? [
        [
          "api-key-redaction",
          "Secrets and API keys",
          "Tokens, private keys, and high-entropy credentials",
          policy.policy.redaction.apiKeys,
        ],
        [
          "payment-card-redaction",
          "Payment cards",
          "Valid payment-card numbers",
          policy.policy.redaction.paymentCards,
        ],
        [
          "email-redaction",
          "Email addresses",
          "Personal and company email addresses",
          policy.policy.redaction.emails,
        ],
        [
          "phone-redaction",
          "Phone numbers",
          "International and local phone numbers",
          policy.policy.redaction.phoneNumbers,
        ],
        [
          "ip-address-redaction",
          "IP addresses",
          "IPv4 network addresses",
          policy.policy.redaction.ipAddresses,
        ],
        [
          "file-path-redaction",
          "File paths",
          "Home, workspace, and local filesystem paths",
          policy.policy.redaction.filePaths,
        ],
      ]
    : [];
  return (
    <section className={styles.enterpriseControls}>
      <article className={styles.enterpriseHero}>
        <div>
          <span>Signed endpoint policy</span>
          <h2>Company-wide defaults</h2>
          <p>
            These settings are distributed to every enrolled desktop. Employees cannot override{" "}
            {MANAGED_CONTROLS_COPY}
          </p>
        </div>
        <em>{policy ? `Policy v${policy.policyVersion}` : "Policy unavailable"}</em>
      </article>

      <div className={styles.enterpriseGrid}>
        <TokenCompressionPanel
          canWrite={canWrite}
          disabled={disabled}
          onChange={onChange}
          policy={policy}
        />

        <article className={styles.controlPanel}>
          <div className={styles.sectionTitle}>
            <div>
              <span>Admin visibility</span>
              <h2>Redacted session previews</h2>
            </div>
          </div>
          <div className={styles.featureControl}>
            <div>
              <strong>Store redacted conversation previews</strong>
              <p>Allow authorized administrators to inspect sanitized employee sessions.</p>
            </div>
            <button
              aria-label="Admin session previews"
              aria-pressed={policy?.policy.content.redactedPreviews ?? false}
              disabled={disabled || !canWrite || !policy}
              onClick={() =>
                onChange("redacted-previews", !(policy?.policy.content.redactedPreviews ?? false))
              }
              type="button"
            >
              <i />
            </button>
          </div>
        </article>
      </div>

      <article className={styles.redactionPanel}>
        <div className={styles.sectionTitle}>
          <div>
            <span>Provider-bound protection</span>
            <h2>What Cutokyo redacts</h2>
          </div>
          <em>Managed by administrators</em>
        </div>
        <div className={styles.redactionGrid}>
          {redactionControls.map(([capability, label, description, enabled]) => (
            <div className={styles.redactionControl} key={capability}>
              <div>
                <strong>{label}</strong>
                <small>{description}</small>
              </div>
              <button
                aria-label={label}
                aria-pressed={enabled}
                disabled={disabled || !canWrite}
                onClick={() => onChange(capability, !enabled)}
                type="button"
              >
                <i />
              </button>
            </div>
          ))}
        </div>
        {!policy ? (
          <p className={styles.empty}>Your role cannot read organization policy.</p>
        ) : null}
      </article>

      <ManagedPluginsPanel
        canWrite={canWrite}
        disabled={pluginDisabled}
        onSave={onPluginSave}
        onTest={onPluginTest}
        onUninstall={onPluginUninstall}
        plugins={plugins}
      />

      <IdentityAdministration canManage={canWrite || permissions.includes("groups:write")} />
    </section>
  );
}

function ManagedPluginsPanel({
  canWrite,
  disabled,
  onSave,
  onTest,
  onUninstall,
  plugins,
}: {
  canWrite: boolean;
  disabled: boolean;
  onSave: (input: {
    enabled: boolean;
    endpoint: string;
    id?: string;
    name: string;
    replaceSecret?: boolean;
    secret?: string;
  }) => Promise<unknown>;
  onTest: (pluginId: string) => Promise<unknown>;
  onUninstall: (pluginId: string) => Promise<unknown>;
  plugins: OrganizationTelemetry["plugins"];
}) {
  const [feedback, setFeedback] = useState<string | null>(null);
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    setFeedback(null);
    const input = {
      enabled: true,
      endpoint: String(data.get("endpoint") ?? ""),
      name: String(data.get("name") ?? "Webhook log stream"),
    };
    const secret = String(data.get("secret") ?? "");
    await onSave(secret ? { ...input, secret } : input);
    form.reset();
    setFeedback("Log stream installed. New organization events will be delivered automatically.");
  };
  const test = async (pluginId: string) => {
    setFeedback(null);
    try {
      await onTest(pluginId);
      setFeedback("Test event delivered successfully.");
    } catch {
      setFeedback("Test delivery failed. Check the endpoint and its server logs.");
    }
  };
  return (
    <article className={styles.managedPlugins}>
      <div className={styles.sectionTitle}>
        <div>
          <span>Organization extensions</span>
          <h2>Managed plugins</h2>
        </div>
        <em>{plugins.filter((plugin) => plugin.enabled).length} active</em>
      </div>
      <p className={styles.panelLead}>
        Install server-vetted plugins once for the whole organization. Log credentials stay
        encrypted on the server and are never distributed to employee desktops.
      </p>
      <div className={styles.pluginCatalogCard}>
        <div>
          <strong>Webhook log stream</strong>
          <small>
            Send content-free operation metadata to your HTTPS collector, SIEM, or workflow.
          </small>
        </div>
        <em>logs:write</em>
      </div>
      <form onSubmit={(event) => void submit(event)}>
        <label>
          Installation name
          <input
            disabled={!canWrite || disabled}
            name="name"
            placeholder="Security event stream"
            required
          />
        </label>
        <label>
          HTTPS endpoint
          <input
            disabled={!canWrite || disabled}
            name="endpoint"
            placeholder="https://logs.example.com/cutokyo"
            required
            type="url"
          />
        </label>
        <label>
          Bearer token <small>optional</small>
          <input
            autoComplete="new-password"
            disabled={!canWrite || disabled}
            name="secret"
            type="password"
          />
        </label>
        <button disabled={!canWrite || disabled} type="submit">
          {!canWrite ? "Read-only" : disabled ? "Working…" : "Install plugin"}
        </button>
      </form>
      {feedback ? <p className={styles.success}>{feedback}</p> : null}
      <div className={styles.managedPluginList}>
        {plugins.map((plugin) => (
          <ManagedPluginRow
            canWrite={canWrite}
            disabled={disabled}
            key={plugin.id}
            onSave={onSave}
            onTest={test}
            onUninstall={onUninstall}
            plugin={plugin}
          />
        ))}
      </div>
    </article>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Maps one preloaded plugin status to its three delegated UI actions.
function ManagedPluginRow({
  canWrite,
  disabled,
  onSave,
  onTest,
  onUninstall,
  plugin,
}: {
  canWrite: boolean;
  disabled: boolean;
  onSave: (input: {
    enabled: boolean;
    endpoint: string;
    id?: string;
    name: string;
    replaceSecret?: boolean;
    secret?: string;
  }) => Promise<unknown>;
  onTest: (pluginId: string) => Promise<unknown>;
  onUninstall: (pluginId: string) => Promise<unknown>;
  plugin: OrganizationTelemetry["plugins"][number];
}) {
  return (
    <div>
      <span className={plugin.lastError ? styles.bad : styles.ok}>
        {plugin.lastError ? "Delivery error" : plugin.enabled ? "Active" : "Paused"}
      </span>
      <div>
        <strong>{plugin.name}</strong>
        <small>{plugin.endpoint}</small>
        <small>
          {compact(plugin.deliveredEvents)} events delivered
          {plugin.lastSuccessAt ? ` · last success ${time(plugin.lastSuccessAt)}` : ""}
        </small>
      </div>
      <div>
        <button disabled={disabled} onClick={() => void onTest(plugin.id)} type="button">
          Test
        </button>
        <button
          disabled={!canWrite || disabled}
          onClick={() =>
            void onSave({
              enabled: !plugin.enabled,
              endpoint: plugin.endpoint,
              id: plugin.id,
              name: plugin.name,
            })
          }
          type="button"
        >
          {plugin.enabled ? "Pause" : "Enable"}
        </button>
        <button
          disabled={!canWrite || disabled}
          onClick={() => void onUninstall(plugin.id)}
          type="button"
        >
          Uninstall
        </button>
      </div>
    </div>
  );
}

function IdentityAdministration({ canManage }: { canManage: boolean }) {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<"dsync" | "sso" | null>(null);
  const open = async (intent: "dsync" | "sso") => {
    setPending(intent);
    setError(null);
    try {
      await openOrganizationIdentitySetup(intent);
    } catch (setupError) {
      setError(setupError instanceof Error ? setupError.message : "Identity setup failed");
      setPending(null);
    }
  };
  return (
    <article className={styles.identityNotice}>
      <div>
        <span>Identity & access</span>
        <h2>Members, SSO, and directories</h2>
        <p>
          Invite employees by email, connect your company sign-in provider, and sync your employee
          directory without leaving Cutokyo setup.
        </p>
        {error ? <strong role="alert">{error}</strong> : null}
      </div>
      <div className={styles.identityActions}>
        <Link href="/admin/employees">Manage employees</Link>
        <button
          disabled={!canManage || pending !== null}
          onClick={() => void open("sso")}
          type="button"
        >
          {pending === "sso" ? "Opening…" : "Configure SSO"}
        </button>
        <button
          disabled={!canManage || pending !== null}
          onClick={() => void open("dsync")}
          type="button"
        >
          {pending === "dsync" ? "Opening…" : "Configure directory sync"}
        </button>
      </div>
    </article>
  );
}

const CONTEXT_DIMENSIONS = [
  ["instructions", "Instructions"],
  ["currentUserInput", "Current input"],
  ["conversationHistory", "Conversation history"],
  ["toolDefinitions", "Tool definitions"],
  ["toolResults", "Tool results"],
  ["mcpDefinitions", "MCP definitions"],
  ["mcpResults", "MCP results"],
  ["retrievedDocuments", "Retrieved documents"],
  ["media", "Media"],
  ["otherContext", "Other context"],
] as const;

function ContextPanel({ context }: { context: OrganizationTelemetry["context"] | undefined }) {
  const dimensions = CONTEXT_DIMENSIONS.map(([key, label]) => ({
    label,
    tokens: context?.[key] ?? 0,
  }));
  const maximum = Math.max(...dimensions.map((dimension) => dimension.tokens), 1);
  const total = dimensions.reduce((sum, dimension) => sum + dimension.tokens, 0);
  return (
    <section className={styles.contextPanel} id="context">
      <div className={styles.sectionTitle}>
        <div>
          <span>Request composition</span>
          <h2>Context analysis</h2>
        </div>
        <em>{compact(total)} attributed tokens</em>
      </div>
      <div className={styles.contextGrid}>
        {dimensions.map((dimension) => (
          <div className={styles.contextRow} key={dimension.label}>
            <span>{dimension.label}</span>
            <i>
              <b style={{ width: `${Math.max(1, (dimension.tokens / maximum) * 100)}%` }} />
            </i>
            <strong>{compact(dimension.tokens)}</strong>
          </div>
        ))}
      </div>
    </section>
  );
}

function EmployeePanel({
  members,
  sessions,
  usage,
}: {
  members: OrganizationTelemetry["members"];
  sessions: OrganizationTelemetry["sessions"];
  usage: OrganizationTelemetry["byUser"];
}) {
  const rows = employeeRows(members, sessions, usage);
  return (
    <section className={styles.activity} id="employees">
      <div className={styles.sectionTitle}>
        <div>
          <span>Organization directory</span>
          <h2>Employee usage</h2>
        </div>
        <em>{rows.length} employees</em>
      </div>
      <div
        aria-label="Scrollable data table"
        className={styles.tableWrap}
        role="region"
        tabIndex={0}
      >
        <table>
          <thead>
            <tr>
              <th>Employee</th>
              <th>Role</th>
              <th>Operations</th>
              <th>Tokens</th>
              <th>Cost</th>
              <th>Sessions</th>
              <th>Last activity</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.id}>
                <td>
                  <span className={styles.employeeIdentity}>
                    <MemberAvatar member={row} />
                    <span>
                      <strong>{row.name}</strong>
                      <small>{row.email ?? row.id}</small>
                    </span>
                  </span>
                </td>
                <td>{row.roles.map(displayName).join(", ") || "Member"}</td>
                <td>{compact(row.events)}</td>
                <td>{compact(row.tokens)}</td>
                <td>{usd(row.costNanosUsd)}</td>
                <td>{row.sessions}</td>
                <td>{row.lastActivity ? time(row.lastActivity) : "No activity"}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {!rows.length ? (
          <p className={styles.empty}>No organization employees are visible.</p>
        ) : null}
      </div>
    </section>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Owns transient form feedback and delegates the invitation mutation to the dashboard model.
function InviteEmployeePanel({
  canInvite,
  disabled,
  invitations,
  onInvite,
}: {
  canInvite: boolean;
  disabled: boolean;
  invitations: OrganizationTelemetry["invitations"];
  onInvite: (input: { email: string; role: string }) => Promise<unknown>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [sentTo, setSentTo] = useState<string | null>(null);
  const pendingInvitations = invitations.filter((invitation) => invitation.state === "pending");
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const email = String(data.get("email") ?? "").trim();
    const role = String(data.get("role") ?? "member");
    setError(null);
    setSentTo(null);
    try {
      await onInvite({ email, role });
      form.reset();
      setSentTo(email);
    } catch (inviteError) {
      setError(inviteError instanceof Error ? inviteError.message : "Invitation failed");
    }
  };
  return (
    <article className={styles.invitePanel}>
      <div className={styles.sectionTitle}>
        <div>
          <span>Team access</span>
          <h2>Invite an employee</h2>
        </div>
        <em>{pendingInvitations.length} pending</em>
      </div>
      <form onSubmit={(event) => void submit(event)}>
        <label>
          Work email
          <input
            autoComplete="email"
            disabled={!canInvite || disabled}
            name="email"
            placeholder="employee@company.com"
            required
            type="email"
          />
        </label>
        <label>
          Role
          <select defaultValue="member" disabled={!canInvite || disabled} name="role">
            <option value="member">Member</option>
            <option value="admin">Administrator</option>
          </select>
        </label>
        <button disabled={!canInvite || disabled} type="submit">
          {disabled ? "Sending…" : "Send invitation"}
        </button>
      </form>
      {!canInvite ? <p className={styles.empty}>Your role cannot invite employees.</p> : null}
      {error ? <p className={styles.error}>{error}</p> : null}
      {sentTo ? <p className={styles.success}>Invitation sent to {sentTo}.</p> : null}
      {pendingInvitations.length ? (
        <div className={styles.invitationList}>
          {pendingInvitations.slice(0, 8).map((invitation) => (
            <span key={invitation.id}>
              <strong>{invitation.email}</strong>
              <small>
                {displayName(invitation.role ?? "member")} · expires {time(invitation.expiresAt)}
              </small>
            </span>
          ))}
        </div>
      ) : null}
    </article>
  );
}

function employeeRows(
  members: OrganizationTelemetry["members"],
  sessions: OrganizationTelemetry["sessions"],
  usage: OrganizationTelemetry["byUser"],
) {
  const membersById = new Map(members.map((member) => [member.id, member]));
  const usageById = new Map(usage.map((item) => [item.name, item]));
  const ids = new Set([...membersById.keys(), ...usageById.keys()]);
  return [...ids]
    .map((id) => {
      const member = membersById.get(id);
      const dimension = usageById.get(id);
      const employeeSessions = sessions.filter((session) => session.userId === id);
      return {
        costNanosUsd: dimension?.costNanosUsd ?? 0,
        email: member?.email,
        events: dimension?.events ?? 0,
        id,
        lastActivity: employeeSessions[0]?.lastActivityAt,
        name: member?.name ?? id,
        profilePictureUrl: member?.profilePictureUrl ?? null,
        roles: member?.roles ?? [],
        sessions: employeeSessions.length,
        tokens: dimension?.tokens ?? 0,
      };
    })
    .sort((left, right) => right.tokens - left.tokens || left.name.localeCompare(right.name));
}

// eslint-disable-next-line local/no-complex-business-logic -- Renders permission, loading, empty, and selected-session states supplied by the dashboard model.
function SessionPanel({
  canRead,
  detail,
  error,
  loading,
  members,
  onSelect,
  selectedId,
  sessions,
}: {
  canRead: boolean;
  detail: OrganizationSessionDetail | null;
  error: string | null;
  loading: boolean;
  members: OrganizationTelemetry["members"];
  onSelect: (id: string) => void;
  selectedId: string | null;
  sessions: OrganizationTelemetry["sessions"];
}) {
  const [employeeId, setEmployeeId] = useState("all");
  const [harness, setHarness] = useState("all");
  const [query, setQuery] = useState("");
  const harnesses = useMemo(
    () => [...new Set(sessions.map((session) => session.harness))].sort(),
    [sessions],
  );
  const filteredSessions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return sessions.filter((session) => {
      if (employeeId !== "all" && session.userId !== employeeId) return false;
      if (harness !== "all" && session.harness !== harness) return false;
      if (!needle) return true;
      const member = members.find((candidate) => candidate.id === session.userId);
      return [
        session.title,
        session.project,
        session.workspace,
        session.provider,
        session.harness,
        member?.email,
        member?.name,
        ...session.models,
      ]
        .filter((value): value is string => Boolean(value))
        .some((value) => value.toLowerCase().includes(needle));
    });
  }, [employeeId, harness, members, query, sessions]);
  const selectedIsVisible = filteredSessions.some((session) => session.id === selectedId);

  return (
    <section className={styles.sessionPanel} id="sessions">
      <div className={styles.sectionTitle}>
        <div>
          <span>Redacted organization history</span>
          <h2>Employee sessions</h2>
        </div>
        <em>{sessions.length} sessions</em>
      </div>
      {!canRead ? (
        <p className={styles.empty}>Your role does not grant sessions:read.</p>
      ) : (
        <>
          <div className={styles.sessionFilters}>
            <label className={styles.sessionSearch}>
              <span>Search</span>
              <input
                aria-label="Search employee sessions"
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Project, model, workspace, or email…"
                type="search"
                value={query}
              />
            </label>
            <label>
              <span>Employee</span>
              <select
                aria-label="Filter sessions by employee"
                onChange={(event) => setEmployeeId(event.target.value)}
                value={employeeId}
              >
                <option value="all">All employees</option>
                {members.map((member) => (
                  <option key={member.id} value={member.id}>
                    {member.email}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span>Harness</span>
              <select
                aria-label="Filter sessions by harness"
                onChange={(event) => setHarness(event.target.value)}
                value={harness}
              >
                <option value="all">All harnesses</option>
                {harnesses.map((item) => (
                  <option key={item} value={item}>
                    {displayName(item)}
                  </option>
                ))}
              </select>
            </label>
            <strong>{filteredSessions.length} results</strong>
          </div>
          <div className={styles.sessionLayout}>
            <div className={styles.sessionList}>
              {filteredSessions.map((session) => (
                <button
                  aria-pressed={session.id === selectedId}
                  key={session.id}
                  onClick={() => onSelect(session.id)}
                  type="button"
                >
                  <span className={styles.sessionListIdentity}>
                    <MemberAvatar member={findMember(members, session.userId)} />
                    <span>
                      <strong>{session.title}</strong>
                      <small>
                        {memberEmail(members, session.userId)} · {displayName(session.harness)}
                      </small>
                      <small>{session.models.join(", ") || displayName(session.provider)}</small>
                    </span>
                  </span>
                  <span className={styles.sessionListMetrics}>
                    <em>{compact(session.totalTokens)} tokens</em>
                    <small>{time(session.lastActivityAt)}</small>
                  </span>
                </button>
              ))}
              {!filteredSessions.length ? (
                <p className={styles.empty}>No employee sessions match these filters.</p>
              ) : null}
            </div>
            <div className={styles.sessionDetail}>
              {loading ? <p className={styles.loading}>Loading redacted session…</p> : null}
              {error ? <p className={styles.error}>{error}</p> : null}
              {detail && !loading && selectedIsVisible ? (
                <SessionDetail detail={detail} members={members} />
              ) : null}
              {!loading && filteredSessions.length > 0 && !selectedIsVisible ? (
                <p className={styles.sessionPrompt}>Select a session from the filtered results.</p>
              ) : null}
            </div>
          </div>
        </>
      )}
    </section>
  );
}

function SessionDetail({
  detail,
  members,
}: {
  detail: OrganizationSessionDetail;
  members: OrganizationTelemetry["members"];
}) {
  return (
    <>
      <header className={styles.sessionHeader}>
        <div>
          <span>
            {memberName(members, detail.userId)} · {memberEmail(members, detail.userId)}
          </span>
          <h3>{detail.title}</h3>
          <p>
            {displayName(detail.harness)} · {detail.models.join(", ") || "Unknown model"}
          </p>
        </div>
        <strong>{compact(detail.totalTokens)} tokens</strong>
      </header>
      <div className={styles.sessionMetrics}>
        <Metric label="Input" value={compact(detail.inputTokens)} />
        <Metric label="Output" value={compact(detail.outputTokens)} />
        <Metric label="Cached" value={compact(detail.cachedTokens)} />
        <Metric label="Cost" value={usd(detail.estimatedCostNanosUsd)} />
      </div>
      <SessionContextComposition context={detail.context} />
      <section className={styles.sessionOperations}>
        <div className={styles.sectionTitle}>
          <div>
            <span>Bounded operation detail</span>
            <h2>Session operations</h2>
          </div>
          <em>
            {detail.operationItems.length} of {detail.operations}
            {detail.operationItemsTruncated ? " · truncated" : ""}
          </em>
        </div>
        <div className={styles.operationList}>
          {detail.operationItems.map((operation) => (
            <OrganizationOperationCard
              key={`${operation.deviceId}-${operation.traceId}`}
              members={members}
              operation={operation}
            />
          ))}
        </div>
      </section>
      <div className={styles.timeline}>
        {detail.messages.map((message, index) => (
          <article key={`${message.traceId}-${index}`}>
            <header>
              <strong>{message.name ?? displayName(message.role)}</strong>
              <span>
                {displayName(message.kind)} · {time(message.timestamp)}
              </span>
            </header>
            <pre>{message.content}</pre>
          </article>
        ))}
        {!detail.messages.length ? (
          <p className={styles.empty}>
            This session has metadata only. Enable admin session previews in policy for future
            traffic.
          </p>
        ) : null}
      </div>
    </>
  );
}

function SessionContextComposition({ context }: { context: OrganizationSessionDetail["context"] }) {
  const dimensions = CONTEXT_DIMENSIONS.map(([key, label]) => ({
    label,
    tokens: context?.[key] ?? 0,
  }));
  const total = dimensions.reduce((sum, dimension) => sum + dimension.tokens, 0);
  return (
    <section className={styles.sessionContext}>
      <div className={styles.sectionTitle}>
        <div>
          <span>Request composition</span>
          <h2>Context composition</h2>
        </div>
        <em>{compact(total)} attributed tokens</em>
      </div>
      <div className={styles.compositionBar} aria-label={`${total} attributed context tokens`}>
        {dimensions.map((dimension) => (
          <i
            key={dimension.label}
            style={{
              width: `${total ? (dimension.tokens / total) * 100 : 0}%`,
            }}
            title={`${dimension.label}: ${compact(dimension.tokens)}`}
          />
        ))}
      </div>
      <div className={styles.compositionLegend}>
        {dimensions
          .filter((dimension) => dimension.tokens > 0)
          .map((dimension) => (
            <span key={dimension.label}>
              <small>{dimension.label}</small>
              <strong>{compact(dimension.tokens)}</strong>
            </span>
          ))}
      </div>
    </section>
  );
}

function TelemetryScopePicker({
  groups,
  members,
  onChange,
  scope,
  users,
}: {
  groups: OrganizationTelemetry["groups"];
  members: OrganizationTelemetry["members"];
  onChange: (scope: TelemetryScope) => void;
  scope: TelemetryScope;
  users: TelemetryDimension[];
}) {
  const value = scope.kind === "all" ? "all" : `${scope.kind}:${scope.id}`;
  return (
    <label className={styles.scopePicker}>
      <span>Telemetry scope</span>
      <select
        aria-label="Telemetry scope"
        onChange={(event) => {
          const [kind, id] = event.target.value.split(":", 2);
          if (kind === "group" && id) {
            const group = groups.find((candidate) => candidate.id === id);
            onChange({ id, kind, label: group?.name ?? id });
          } else if (kind === "user" && id) {
            const member = members.find((candidate) => candidate.id === id);
            onChange({ id, kind, label: member?.email ?? id });
          } else {
            onChange({ kind: "all", label: "Everyone" });
          }
        }}
        value={value}
      >
        <option value="all">Everyone</option>
        {groups.map((group) => (
          <option key={group.id} value={`group:${group.id}`}>
            Group · {group.name}
          </option>
        ))}
        {users.map((user) => (
          <option key={user.name} value={`user:${user.name}`}>
            User · {members.find((member) => member.id === user.name)?.email ?? user.name}
          </option>
        ))}
      </select>
      <strong>{scope.label}</strong>
    </label>
  );
}

function DevicePanel({ devices }: { devices: OrganizationTelemetry["devices"] }) {
  return (
    <article className={styles.controlPanel}>
      <div className={styles.sectionTitle}>
        <h2>Enrolled devices</h2>
        <em>{devices.length}</em>
      </div>
      {devices.slice(0, 5).map((device) => (
        <div className={styles.listRow} key={device.id}>
          <div>
            <strong>{device.name}</strong>
            <small>
              {device.platform} · {device.appVersion}
            </small>
          </div>
          <span className={device.revokedAt ? styles.bad : styles.ok}>
            {device.revokedAt ? "revoked" : "active"}
          </span>
        </div>
      ))}
      {!devices.length ? <p className={styles.empty}>No visible devices for this role.</p> : null}
    </article>
  );
}

function AuditPanel({ events }: { events: OrganizationTelemetry["auditEvents"] }) {
  return (
    <article className={styles.controlPanel}>
      <div className={styles.sectionTitle}>
        <h2>Immutable audit</h2>
        <em>{events.length}</em>
      </div>
      {events.slice(0, 5).map((event) => (
        <div className={styles.listRow} key={event.id}>
          <div>
            <strong>{displayName(event.action)}</strong>
            <small>
              {event.actorId} · {time(event.createdAt)}
            </small>
          </div>
          <span>{event.targetId}</span>
        </div>
      ))}
      {!events.length ? (
        <p className={styles.empty}>No visible audit events for this role.</p>
      ) : null}
    </article>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Owns transient form fields and delegates all group mutations to FastAPI commands.
function GroupPanel({
  canWrite,
  disabled,
  groups,
  members,
  onDelete,
  onSave,
}: {
  canWrite: boolean;
  disabled: boolean;
  groups: OrganizationTelemetry["groups"];
  members: OrganizationTelemetry["members"];
  onDelete: (groupId: string) => Promise<unknown>;
  onSave: (input: { id?: string; memberIds: string[]; name: string }) => Promise<unknown>;
}) {
  const [editingId, setEditingId] = useState<string | undefined>();
  const [memberQuery, setMemberQuery] = useState("");
  const [name, setName] = useState("");
  const [selectedMemberIds, setSelectedMemberIds] = useState<string[]>([]);
  const visibleMembers = useMemo(() => {
    const query = memberQuery.trim().toLowerCase();
    if (!query) return members;
    return members.filter((member) =>
      [member.email, member.name, ...member.roles].some((value) =>
        value.toLowerCase().includes(query),
      ),
    );
  }, [memberQuery, members]);
  const membersById = new Map(members.map((member) => [member.id, member]));

  const reset = () => {
    setEditingId(undefined);
    setMemberQuery("");
    setName("");
    setSelectedMemberIds([]);
  };
  const edit = (group: OrganizationTelemetry["groups"][number]) => {
    setEditingId(group.id);
    setMemberQuery("");
    setSelectedMemberIds(group.memberIds);
    setName(group.name);
  };
  const toggleMember = (memberId: string) => {
    setSelectedMemberIds((current) =>
      current.includes(memberId)
        ? current.filter((candidate) => candidate !== memberId)
        : [...current, memberId],
    );
  };
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    await onSave(
      editingId
        ? { id: editingId, memberIds: selectedMemberIds, name }
        : { memberIds: selectedMemberIds, name },
    );
    reset();
  };

  return (
    <article className={styles.groupPanel}>
      <div className={styles.sectionTitle}>
        <div>
          <span>Email-based membership</span>
          <h2>Organization groups</h2>
        </div>
        <em>{groups.length}</em>
      </div>
      <div className={styles.groupLayout}>
        <div>
          {groups.map((group) => (
            <div className={styles.groupRow} key={group.id}>
              <button disabled={!canWrite || disabled} onClick={() => edit(group)} type="button">
                <strong>{group.name}</strong>
                <small>
                  {group.memberIds.length
                    ? group.memberIds
                        .slice(0, 3)
                        .map((memberId) => membersById.get(memberId)?.email ?? "Former member")
                        .join(", ")
                    : "No members"}
                  {group.memberIds.length > 3 ? ` +${group.memberIds.length - 3}` : ""}
                </small>
              </button>
              <button
                aria-label={`Delete ${group.name}`}
                disabled={!canWrite || disabled}
                onClick={() => void onDelete(group.id)}
                type="button"
              >
                Delete
              </button>
            </div>
          ))}
          {!groups.length ? (
            <p className={styles.empty}>No groups yet. Create one from real organization users.</p>
          ) : null}
        </div>
        <form onSubmit={(event) => void submit(event)}>
          <label>
            Group name
            <input
              disabled={!canWrite || disabled}
              maxLength={120}
              onChange={(event) => setName(event.target.value)}
              required
              value={name}
            />
          </label>
          <fieldset className={styles.memberPicker} disabled={!canWrite || disabled}>
            <legend>Members by email</legend>
            <input
              aria-label="Search organization members"
              disabled={!canWrite || disabled}
              onChange={(event) => setMemberQuery(event.target.value)}
              placeholder="Search by name, email, or role…"
              type="search"
              value={memberQuery}
            />
            <div className={styles.selectedMembers}>
              {selectedMemberIds.map((memberId) => {
                const member = membersById.get(memberId);
                return member ? (
                  <button key={memberId} onClick={() => toggleMember(memberId)} type="button">
                    {member.email} <span>×</span>
                  </button>
                ) : null;
              })}
              {!selectedMemberIds.length ? <small>No members selected</small> : null}
            </div>
            <div className={styles.memberOptions}>
              {visibleMembers.map((member) => {
                const selected = selectedMemberIds.includes(member.id);
                return (
                  <button
                    aria-pressed={selected}
                    key={member.id}
                    onClick={() => toggleMember(member.id)}
                    type="button"
                  >
                    <MemberAvatar member={member} />
                    <span>
                      <strong>{member.name}</strong>
                      <small>{member.email}</small>
                    </span>
                    <em>{selected ? "Selected" : "Add"}</em>
                  </button>
                );
              })}
              {!visibleMembers.length ? (
                <p className={styles.empty}>No organization member matches this search.</p>
              ) : null}
            </div>
          </fieldset>
          <div>
            <button disabled={!canWrite || disabled} type="submit">
              {disabled ? "Saving…" : editingId ? "Update group" : "Create group"}
            </button>
            {editingId ? (
              <button disabled={disabled} onClick={reset} type="button">
                Cancel
              </button>
            ) : null}
          </div>
        </form>
      </div>
    </article>
  );
}

function Metric({
  accent = false,
  label,
  value,
}: {
  accent?: boolean;
  label: string;
  value: string;
}) {
  return (
    <article className={accent ? styles.metricAccent : styles.metric}>
      <span>{label}</span>
      <strong>{value}</strong>
    </article>
  );
}

function Breakdown({
  dimensions,
  label = (item) => displayName(item.name),
  title,
}: {
  dimensions: TelemetryDimension[];
  label?: (item: TelemetryDimension) => string;
  title: string;
}) {
  const maximum = Math.max(...dimensions.map((item) => item.tokens), 1);
  return (
    <article aria-label={title} className={styles.breakdown}>
      <div className={styles.sectionTitle}>
        <h2>{title}</h2>
        <em>{dimensions.length}</em>
      </div>
      <div>
        {dimensions.slice(0, 6).map((item) => (
          <div className={styles.barRow} key={item.name}>
            <span>{label(item)}</span>
            <i>
              <b style={{ width: `${Math.max(2, (item.tokens / maximum) * 100)}%` }} />
            </i>
            <strong>{compact(item.tokens)}</strong>
          </div>
        ))}
        {!dimensions.length ? <p className={styles.empty}>Waiting for traffic.</p> : null}
      </div>
    </article>
  );
}

function compact(value: number) {
  return new Intl.NumberFormat("en", { notation: value >= 10_000 ? "compact" : "standard" }).format(
    value,
  );
}

function usd(nanos: number) {
  return new Intl.NumberFormat("en", {
    currency: "USD",
    maximumFractionDigits: nanos < 10_000_000 ? 6 : 2,
    style: "currency",
  }).format(nanos / 1_000_000_000);
}

function time(value: string) {
  return new Intl.DateTimeFormat("en", {
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    month: "short",
  }).format(new Date(value));
}

function shortDate(value: string) {
  return new Intl.DateTimeFormat("en", { day: "2-digit", month: "short" }).format(new Date(value));
}

function reportOptions(dimensions: TelemetryDimension[], selected: string) {
  return [...new Set([...dimensions.map((dimension) => dimension.name), selected])]
    .filter((value) => value !== "all")
    .sort((left, right) => left.localeCompare(right));
}

function exportTelemetryCsv(
  events: OrganizationTelemetry["recentEvents"],
  members: OrganizationTelemetry["members"],
) {
  const headers = [
    "timestamp",
    "employee",
    "harness",
    "provider",
    "model",
    "tokens",
    "cost_usd",
    "outcome",
    "duration_ms",
    "workspace",
    "project",
    "trace_id",
  ];
  const rows = events.map((event) => [
    event.timestamp,
    memberEmail(members, event.userId),
    event.harness,
    event.provider,
    event.model ?? "",
    event.totalTokens,
    event.costNanosUsd / 1_000_000_000,
    event.outcome,
    event.durationMs,
    event.workspace ?? "",
    event.project ?? "",
    event.traceId,
  ]);
  const csv = [headers, ...rows].map((row) => row.map(csvCell).join(",")).join("\n");
  const link = document.createElement("a");
  link.href = URL.createObjectURL(new Blob([csv], { type: "text/csv;charset=utf-8" }));
  link.download = `cutokyo-report-${new Date().toISOString().slice(0, 10)}.csv`;
  link.click();
  URL.revokeObjectURL(link.href);
}

function csvCell(value: number | string) {
  const text = String(value);
  const safe = /^[+\-=@]/.test(text) ? `'${text}` : text;
  return `"${safe.replaceAll('"', '""')}"`;
}

function displayName(value: string) {
  return value.replaceAll("-", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function MemberAvatar({
  member,
}: {
  member:
    | {
        email?: string | null | undefined;
        name: string;
        profilePictureUrl?: string | null | undefined;
      }
    | undefined;
}) {
  const fallback = (member?.name || member?.email || "U").at(0)?.toUpperCase() ?? "U";
  return (
    <span className={styles.memberAvatar}>
      {member?.profilePictureUrl ? (
        // eslint-disable-next-line @next/next/no-img-element -- Employee photo hosts are configured by each organization.
        <img alt="" referrerPolicy="no-referrer" src={member.profilePictureUrl} />
      ) : (
        fallback
      )}
    </span>
  );
}

function findMember(members: OrganizationTelemetry["members"], userId: string) {
  return members.find((member) => member.id === userId);
}

function memberName(members: OrganizationTelemetry["members"], userId: string) {
  return findMember(members, userId)?.name ?? userId;
}

function memberEmail(members: OrganizationTelemetry["members"], userId: string) {
  return findMember(members, userId)?.email ?? userId;
}

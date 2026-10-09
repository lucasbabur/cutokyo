"use client";

import Image from "next/image";
import { useId, useMemo, useState, useSyncExternalStore } from "react";

import { tokenSavingEnabled } from "@/shared/config/features";

import {
  CATEGORY_LABELS,
  initialDesktopView,
  navigateDesktopView,
  providerAlignedContext,
  serverDesktopView,
  subscribeDesktopView,
  useDesktopDashboard,
} from "./model";
import styles from "./view.module.css";

import type {
  AttributedCategories,
  DesktopSnapshot,
  DesktopView,
  FeatureSettings,
  LocalSession,
  OperationEvent,
} from "./model";
import type { CSSProperties, ReactNode } from "react";

type DesktopDashboardViewProps = {
  initialData?: DesktopSnapshot;
};

const DEFAULT_SETTINGS: FeatureSettings = {
  apiKeyRedaction: true,
  autoConnect: true,
  compression: false,
  conversationRecording: true,
  emailRedaction: true,
  filePathRedaction: true,
  ipAddressRedaction: true,
  observability: true,
  organizationSync: true,
  otlpExport: true,
  paymentCardRedaction: true,
  phoneNumberRedaction: true,
  plugins: false,
  providerMutation: false,
};

const ALL_SETTINGS: { detail: string; key: keyof FeatureSettings; label: string }[] = [
  {
    key: "observability",
    label: "Local observability",
    detail: "Record encrypted metadata-only operation events.",
  },
  {
    key: "conversationRecording",
    label: "Session conversation",
    detail: "Store encrypted redacted request and assistant previews.",
  },
  {
    key: "apiKeyRedaction",
    label: "API key redaction",
    detail: "Remove tokens, private keys, and high-entropy secrets.",
  },
  {
    key: "paymentCardRedaction",
    label: "Payment card redaction",
    detail: "Remove valid payment card numbers before forwarding.",
  },
  {
    key: "emailRedaction",
    label: "Email redaction",
    detail: "Remove email addresses from provider-bound payloads.",
  },
  {
    key: "phoneNumberRedaction",
    label: "Phone redaction",
    detail: "Remove phone numbers from provider-bound payloads.",
  },
  {
    key: "ipAddressRedaction",
    label: "IP address redaction",
    detail: "Remove IPv4 addresses from provider-bound payloads.",
  },
  {
    key: "filePathRedaction",
    label: "File path redaction",
    detail: "Protect local workspace and home-directory paths.",
  },
  {
    key: "otlpExport",
    label: "OpenTelemetry export",
    detail: "Forward the durable local spool to your configured collector.",
  },
  {
    key: "organizationSync",
    label: "Organization sync",
    detail: "Upload usage plus policy-governed redacted previews to your organization.",
  },
  {
    key: "autoConnect",
    label: "Automatic connection",
    detail: "Detect and connect supported local AI tools.",
  },
  {
    key: "plugins",
    label: "Proxy plugins",
    detail: "Run configured Python or TypeScript plugins after built-in redaction.",
  },
  {
    key: "compression",
    label: "Token compression",
    detail: "Optimize eligible tool results when you explicitly enable it.",
  },
];

const SETTINGS = ALL_SETTINGS.filter(
  (setting) => tokenSavingEnabled || setting.key !== "compression",
);

const ORGANIZATION_MANAGED_SETTINGS = new Set<keyof FeatureSettings>([
  "apiKeyRedaction",
  "compression",
  "conversationRecording",
  "emailRedaction",
  "filePathRedaction",
  "ipAddressRedaction",
  "paymentCardRedaction",
  "phoneNumberRedaction",
]);

const EMPTY_CONTEXT: AttributedCategories = {
  conversationHistory: 0,
  currentUserInput: 0,
  instructions: 0,
  mcpDefinitions: 0,
  mcpResults: 0,
  media: 0,
  otherContext: 0,
  retrievedDocuments: 0,
  toolDefinitions: 0,
  toolResults: 0,
};

// eslint-disable-next-line local/no-complex-business-logic -- Selects render-only dashboard panels; data orchestration lives in the model.
export function DesktopDashboardView({ initialData }: DesktopDashboardViewProps) {
  const {
    action,
    error,
    loadOperations,
    operationCorrelation,
    runAction,
    snapshot,
    summary,
    updateSettings,
  } = useDesktopDashboard(initialData);
  const routeView = useSyncExternalStore(
    subscribeDesktopView,
    initialDesktopView,
    serverDesktopView,
  );
  const view = routeView;
  const loading = snapshot === null && error === null;

  return (
    <main aria-busy={loading} className={styles.shell}>
      <Sidebar loading={loading} onNavigate={navigateDesktopView} snapshot={snapshot} view={view} />
      <section className={styles.workspace}>
        <Topbar action={action} onAction={runAction} snapshot={snapshot} view={view} />
        {loading ? (
          <div className={styles.loadingBanner} role="status">
            Loading encrypted endpoint state…
          </div>
        ) : null}
        {error ? (
          <div className={styles.errorBanner} role="alert">
            {error}
          </div>
        ) : null}
        {view === "overview" ? (
          <Overview
            action={action}
            onAction={runAction}
            onLoadOperations={loadOperations}
            onUpdate={updateSettings}
            operationCorrelation={operationCorrelation}
            snapshot={snapshot}
            summary={summary}
          />
        ) : null}
        {view === "sessions" ? <SessionsView sessions={snapshot?.sessions ?? []} /> : null}
        {view === "context" ? (
          <ContextView
            context={summary.categories}
            organizationConnected={snapshot?.session.organizationConnected === true}
            sessions={snapshot?.sessions ?? []}
            totalTokens={summary.totalTokens}
          />
        ) : null}
        {view === "plugins" ? <PluginsView snapshot={snapshot} /> : null}
        {view === "settings" ? (
          <SettingsView
            action={action}
            onAction={runAction}
            onUpdate={updateSettings}
            settings={snapshot?.settings ?? DEFAULT_SETTINGS}
            snapshot={snapshot}
          />
        ) : null}
      </section>
    </main>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Maps navigation and endpoint status to presentational labels only.
function Sidebar({
  loading,
  onNavigate,
  snapshot,
  view,
}: {
  loading: boolean;
  onNavigate: (view: DesktopView) => void;
  snapshot: DesktopSnapshot | null;
  view: DesktopView;
}) {
  const items: { icon: DesktopIconName; key: DesktopView; label: string }[] = [
    { icon: "overview", key: "overview", label: "Overview" },
    { icon: "sessions", key: "sessions", label: "Sessions" },
    { icon: "context", key: "context", label: "Context" },
    { icon: "plugins", key: "plugins", label: "Plugins" },
    { icon: "settings", key: "settings", label: "Controls" },
  ];
  const proxyReady = snapshot?.status.status === "ready";
  const enterprise = snapshot?.session.organizationConnected === true;
  return (
    <aside className={styles.sidebar}>
      <button className={styles.brand} onClick={() => onNavigate("overview")} type="button">
        <Image alt="" height={38} src="/brand/cutokyo-mark.svg" width={38} />
        <span>cutokyo</span>
      </button>
      <p className={styles.sideLabel}>
        {enterprise ? snapshot?.session.organizationName || "Enterprise" : "Personal workspace"}
      </p>
      <nav aria-label="Desktop navigation">
        {items.map((item) => (
          <button
            aria-current={view === item.key ? "page" : undefined}
            aria-label={item.label}
            key={item.key}
            onClick={() => onNavigate(item.key)}
            type="button"
          >
            <DesktopIcon name={item.icon} />
            <span>{item.label}</span>
            {item.key !== "overview" ? <em>→</em> : null}
          </button>
        ))}
      </nav>
      <div className={styles.sidebarFoot}>
        <i
          className={loading ? styles.pendingDot : proxyReady ? styles.liveDot : styles.offlineDot}
        />
        <div>
          <strong>
            {loading ? "Checking proxy" : proxyReady ? "Proxy live" : "Proxy unavailable"}
          </strong>
          <small>{loading ? "Loading endpoint state" : "localhost:49321"}</small>
        </div>
      </div>
    </aside>
  );
}

type DesktopIconName = "context" | "overview" | "plugins" | "sessions" | "settings";

function DesktopIcon({ name }: { name: DesktopIconName }) {
  const paths: Record<DesktopIconName, ReactNode> = {
    context: (
      <>
        <path d="m12 3-8 4 8 4 8-4-8-4Z" />
        <path d="m4 12 8 4 8-4M4 17l8 4 8-4" />
      </>
    ),
    overview: (
      <>
        <rect height="7" rx="1" width="7" x="3" y="3" />
        <rect height="7" rx="1" width="7" x="14" y="3" />
        <rect height="7" rx="1" width="7" x="3" y="14" />
        <rect height="7" rx="1" width="7" x="14" y="14" />
      </>
    ),
    plugins: <path d="M8 3h3v4h2V3h3v4h2v4a6 6 0 0 1-5 5.9V21h-2v-4.1A6 6 0 0 1 6 11V7h2V3Z" />,
    sessions: (
      <>
        <path d="M4 5h16v11H9l-5 4V5Z" />
        <path d="M8 9h8M8 12h5" />
      </>
    ),
    settings: (
      <>
        <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
        <circle cx="16" cy="7" r="2" />
        <circle cx="8" cy="17" r="2" />
      </>
    ),
  };
  return (
    <svg aria-hidden="true" className={styles.navIcon} fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.5">
        {paths[name]}
      </g>
    </svg>
  );
}

function Topbar({
  action,
  onAction,
  snapshot,
  view,
}: {
  action: string | null;
  onAction: (action: "connect" | "deactivate" | "signin" | "signout") => Promise<void>;
  snapshot: DesktopSnapshot | null;
  view: DesktopView;
}) {
  const titles: Record<DesktopView, [string, string]> = {
    overview: ["Live traffic", "Your AI stack, visible."],
    sessions: ["Local index", "Every session. Every harness."],
    context: ["Context anatomy", "See what the model actually receives."],
    plugins: ["Request plugins", "Extend the proxy without losing governance."],
    settings: ["Capability controls", "Nothing hidden. Your rules."],
  };
  return (
    <header className={styles.topbar}>
      <div>
        <p>{titles[view][0]}</p>
        <h1 className={view === "context" ? styles.contextTitle : undefined}>{titles[view][1]}</h1>
      </div>
      <div className={styles.headerActions}>
        {snapshot?.session.authenticated ? (
          <div className={styles.accountActions}>
            <span className={styles.accountIdentity}>
              <small>Signed in as</small>
              <strong>
                {snapshot.session.user?.name ?? snapshot.session.user?.email ?? "Cutokyo user"}
              </strong>
            </span>
            <button
              className={styles.signOutButton}
              disabled={action !== null}
              onClick={() => void onAction("signout")}
              type="button"
            >
              {action === "signout" ? "Signing out…" : "Sign out"}
            </button>
          </div>
        ) : (
          <button
            className={styles.primaryButton}
            disabled={
              snapshot === null || action !== null || snapshot.session.authConfigured === false
            }
            onClick={() => void onAction("signin")}
            type="button"
          >
            {action === "signin" ? "Opening…" : "Sign in"}
          </button>
        )}
      </div>
    </header>
  );
}

// eslint-disable-next-line local/no-complex-business-logic -- Renders precomputed summary and endpoint state without owning business decisions.
function Overview({
  action,
  onAction,
  onLoadOperations,
  onUpdate,
  operationCorrelation,
  snapshot,
  summary,
}: {
  action: string | null;
  onAction: (action: "connect" | "deactivate") => Promise<void>;
  onLoadOperations: (correlation: string, cursor?: number) => Promise<void>;
  onUpdate: (settings: FeatureSettings) => Promise<void>;
  operationCorrelation: string;
  snapshot: DesktopSnapshot | null;
  summary: ReturnType<typeof useDesktopDashboard>["summary"];
}) {
  const [correlation, setCorrelation] = useState(operationCorrelation);
  const settings = snapshot?.settings ?? DEFAULT_SETTINGS;
  const compressionEnabled = settings.compression;
  const compressionManaged = snapshot?.session.organizationConnected === true;
  const storageDegraded = snapshot?.storage.degraded === true;
  const storageDetail = snapshot
    ? [
        `${snapshot.storage.records} records · ${snapshot.storage.keySource}`,
        snapshot.storage.retentionError,
        snapshot.storage.blockedRecords > 0 ? `${snapshot.storage.blockedRecords} blocked` : null,
        snapshot.storage.quarantinedRecords > 0
          ? `${snapshot.storage.quarantinedRecords} quarantined`
          : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "Loading encrypted store status.";
  return (
    <>
      <section className={styles.metrics}>
        <MetricCard
          label="Provider tokens"
          note="Authoritative total"
          tokenValue
          value={compact(summary.totalTokens)}
        />
        {tokenSavingEnabled ? (
          <article className={styles.compressionMetric}>
            <div>
              <span>{compressionEnabled ? "Saved tokens" : "Compression"}</span>
              <strong>
                {compressionEnabled
                  ? compact(snapshot?.metrics.totalEstimatedTokensSaved ?? 0)
                  : "OFF"}
              </strong>
              <small>
                {compressionEnabled
                  ? `${percent(snapshot?.metrics.savingsRatio ?? 0)} reduction`
                  : compressionManaged
                    ? "Disabled by organization default"
                    : "Enable guarded tool-result optimization"}
              </small>
            </div>
            <button
              aria-label={`${compressionEnabled ? "Disable" : "Enable"} Token compression`}
              aria-pressed={compressionEnabled}
              className={styles.switch}
              disabled={snapshot === null || action !== null || compressionManaged}
              onClick={() =>
                void onUpdate({
                  ...settings,
                  compression: !compressionEnabled,
                  providerMutation: !compressionEnabled,
                })
              }
              type="button"
            >
              <i />
            </button>
          </article>
        ) : null}
      </section>

      <section className={styles.dashboardGrid}>
        <article className={styles.heroPanel}>
          <PanelHeader
            eyebrow="Estimated request composition"
            meta={`${compact(summary.attributedTotal)} tokens`}
            title="Context signal"
          />
          <ContextStack context={summary.categories} />
          <div className={styles.contextHighlights}>
            {topContext(summary.categories, 4).map(([key, value]) => (
              <div key={key}>
                <i style={{ "--context-color": contextColor(key) } as CSSProperties} />
                <span>{categoryLabel(key)}</span>
                <strong>{compact(value)}</strong>
              </div>
            ))}
          </div>
          <ContextFindings context={summary.categories} scope="aggregate" />
        </article>

        <article className={styles.connectPanel}>
          <header className={styles.attributionHeader}>
            <div>
              <span>Automatic attribution</span>
              <h2>Traffic sources</h2>
              <p>Detected from real requests and local installation evidence.</p>
            </div>
            <strong>{summary.events}</strong>
            <small>operations</small>
          </header>
          <div className={styles.harnessList}>
            {summary.harnesses.length ? (
              summary.harnesses.map((harness) => (
                <div className={styles.harnessRow} key={harness.name}>
                  <span className={styles.harnessMark}>{initials(harness.name)}</span>
                  <div>
                    <strong>{displayHarness(harness.name)}</strong>
                    <small>{compact(harness.tokens)} provider tokens</small>
                  </div>
                  <em>{harness.operations} ops</em>
                </div>
              ))
            ) : (
              <EmptyState body="Run a supported AI tool. Cutokyo will attribute its first request automatically." />
            )}
          </div>
          <button
            className={styles.connectButton}
            disabled={snapshot === null || action !== null}
            onClick={() => void onAction("connect")}
            type="button"
          >
            {action === "connect" ? "Detecting…" : "Detect & connect tools"}
          </button>
          <button
            className={styles.dangerButton}
            disabled={snapshot === null || action !== null}
            onClick={() => void onAction("deactivate")}
            type="button"
          >
            {action === "deactivate" ? "Restoring…" : "Disconnect and restore"}
          </button>
        </article>
      </section>

      <section className={styles.activityPanel}>
        <PanelHeader
          eyebrow={operationCorrelation ? "Exact correlation" : "Policy-safe telemetry"}
          meta="Newest first"
          title="Operation investigation"
        />
        <form
          className={styles.operationSearch}
          onSubmit={(event) => {
            event.preventDefault();
            void onLoadOperations(correlation);
          }}
        >
          <label className={styles.searchBox}>
            <span>#</span>
            <input
              aria-label="Trace, request, or provider response ID"
              maxLength={240}
              onChange={(event) => setCorrelation(event.target.value)}
              placeholder="Exact trace, request, or provider response ID"
              value={correlation}
            />
          </label>
          <button type="submit">Find exact</button>
          {operationCorrelation || snapshot?.eventsNextCursor ? (
            <button
              onClick={() => {
                setCorrelation("");
                void onLoadOperations("");
              }}
              type="button"
            >
              Reset
            </button>
          ) : null}
        </form>
        <OperationsTable events={snapshot?.events ?? []} />
        {snapshot?.eventsNextCursor ? (
          <button
            className={styles.loadOlderButton}
            onClick={() =>
              void onLoadOperations(operationCorrelation, snapshot.eventsNextCursor ?? undefined)
            }
            type="button"
          >
            Load older operations
          </button>
        ) : null}
      </section>

      <section className={styles.healthGrid}>
        <HealthCard
          label="Local store"
          status={snapshot === null ? "Checking…" : storageDegraded ? "Degraded" : "Encrypted"}
          detail={storageDetail}
          tone={snapshot === null ? "neutral" : storageDegraded ? "bad" : "good"}
        />
        <HealthCard
          label="Redaction"
          status={snapshot === null ? "Checking…" : "Active"}
          detail={
            snapshot === null
              ? "Loading provider-bound privacy policy."
              : `${summary.redactions} sensitive values removed before provider forwarding.`
          }
          tone={snapshot === null ? "neutral" : "good"}
        />
        <HealthCard
          label="OpenTelemetry"
          status={
            snapshot === null
              ? "Checking…"
              : snapshot.exporter.enabled
                ? "Export active"
                : "Export disabled"
          }
          detail={
            snapshot === null
              ? "Loading exporter status."
              : snapshot.exporter.enabled
                ? `${snapshot.exporter.protocol} · ${snapshot.exporter.pendingEvents} pending`
                : "Configure an OTLP collector when you are ready."
          }
          tone={snapshot === null ? "neutral" : snapshot.exporter.lastError ? "bad" : "good"}
        />
        <HealthCard
          label="Organization"
          status={
            snapshot === null
              ? "Checking…"
              : snapshot.organization?.connected
                ? "Connected"
                : "Local mode"
          }
          detail={
            snapshot === null
              ? "Loading organization connection state."
              : snapshot.organization?.connected
                ? `${snapshot.session.organizationName ? `${snapshot.session.organizationName} · ` : ""}${snapshot.organization.uploadedEvents} events uploaded`
                : "Sign in to connect this endpoint to your organization."
          }
          tone={snapshot === null ? "neutral" : snapshot.organization?.lastError ? "bad" : "good"}
        />
      </section>
    </>
  );
}

function SessionsView({ sessions }: { sessions: LocalSession[] }) {
  const [query, setQuery] = useState("");
  const [harness, setHarness] = useState("all");
  const harnesses = useMemo(
    () => [...new Set(sessions.map((session) => session.harness))],
    [sessions],
  );
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return sessions.filter((session) => {
      if (harness !== "all" && session.harness !== harness) return false;
      if (!needle) return true;
      return [
        session.title,
        session.harness,
        session.provider,
        session.workspace,
        session.project,
        ...session.models,
      ]
        .filter(Boolean)
        .some((value) => value?.toLowerCase().includes(needle));
    });
  }, [harness, query, sessions]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = filtered.find((session) => session.id === selectedId) ?? filtered[0] ?? null;

  return (
    <section className={styles.sessionsLayout}>
      <div className={styles.sessionBrowser}>
        <div className={styles.searchBox}>
          <span>⌕</span>
          <input
            aria-label="Search local sessions"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search project, model, workspace…"
            type="search"
            value={query}
          />
        </div>
        <div className={styles.filterRow}>
          {["all", ...harnesses].map((item) => (
            <button
              aria-pressed={harness === item}
              key={item}
              onClick={() => setHarness(item)}
              type="button"
            >
              {item === "all" ? "All" : displayHarness(item)}
            </button>
          ))}
        </div>
        <p className={styles.resultCount}>{filtered.length} local sessions</p>
        <div className={styles.sessionList}>
          {filtered.map((session) => (
            <button
              aria-pressed={selected?.id === session.id}
              key={session.id}
              onClick={() => setSelectedId(session.id)}
              type="button"
            >
              <ProviderMark session={session} />
              <div>
                <strong>{session.title}</strong>
                <small>
                  {displayHarness(session.harness)} · {relativeTime(session.lastActivityAt)}
                </small>
              </div>
              <em>{compact(session.totalTokens)}</em>
            </button>
          ))}
          {!filtered.length ? <EmptyState body="No sessions match this search." /> : null}
        </div>
      </div>
      <div className={styles.sessionDetail}>
        {selected ? (
          <SessionDetail session={selected} />
        ) : (
          <EmptyState body="Your first attributed session will appear here." />
        )}
      </div>
    </section>
  );
}

function SessionDetail({ session }: { session: LocalSession }) {
  const context = providerAlignedContext(session.context, session.inputTokens);
  return (
    <>
      <header className={styles.sessionTitle}>
        <div className={styles.sessionIdentity}>
          <ProviderMark detail session={session} />
          <div>
            <p>
              {displayHarness(session.harness)} · {displayProvider(session.provider)}
            </p>
            <h2>{session.title}</h2>
          </div>
        </div>
        <span>{session.operations} operations</span>
      </header>
      <div className={styles.sessionMeta}>
        <span>
          Model<strong>{session.models.join(", ") || "Unknown"}</strong>
        </span>
        <span>
          Workspace<strong>{session.workspace ?? session.project ?? "Unassigned"}</strong>
        </span>
        <span>
          Last activity<strong>{new Date(session.lastActivityAt).toLocaleString()}</strong>
        </span>
      </div>
      <article className={styles.contextDetail}>
        <PanelHeader
          eyebrow="Across this session"
          meta={`${compact(contextTotal(context))} attributed input tokens`}
          title="Content composition"
        />
        <ContextStack context={context} />
        <div className={styles.categoryGrid}>
          {CATEGORY_LABELS.map(([key, label]) => (
            <CategoryRow
              key={key}
              label={label}
              max={Math.max(...Object.values(context), 1)}
              value={context[key]}
            />
          ))}
        </div>
        <ContextFindings context={context} scope="session" />
      </article>
      <section className={styles.sessionTokenGrid}>
        <MetricCard label="Total" note="Provider tokens" value={compact(session.totalTokens)} />
        <MetricCard label="Input" note="Prompt + context" value={compact(session.inputTokens)} />
        <MetricCard label="Output" note="Model output" value={compact(session.outputTokens)} />
        <MetricCard label="Cache hits" note="Read tokens" value={compact(session.cachedTokens)} />
      </section>
      <article className={styles.conversationDetail}>
        <PanelHeader
          eyebrow="Encrypted on this device"
          meta={`${session.messages.length} timeline items`}
          title="Conversation"
        />
        {session.messages.length ? (
          <div className={styles.messageTimeline}>
            {/* eslint-disable-next-line local/no-complex-business-logic -- Renders one stored message variant without mutating session state. */}
            {session.messages.map((message, index) => {
              const preview = messagePreview(message.content);
              const kind = message.kind ?? "message";
              const toolName = displayToolName(message.name);
              const label =
                kind === "tool_call"
                  ? `Tool call · ${toolName}`
                  : kind === "tool_result"
                    ? `Tool result · ${toolName}`
                    : message.role === "assistant"
                      ? "Assistant"
                      : "You";
              const messageClass =
                kind === "tool_call"
                  ? styles.toolCallMessage
                  : kind === "tool_result"
                    ? styles.toolResultMessage
                    : message.role === "assistant"
                      ? styles.assistantMessage
                      : styles.userMessage;
              return (
                <article
                  className={messageClass}
                  key={`${message.traceId}-${kind}-${message.role}-${index}`}
                >
                  {preview.expandable ? (
                    <details aria-label={`Full ${label} message at ${time(message.timestamp)}`}>
                      <summary aria-label={`Expand ${label} message at ${time(message.timestamp)}`}>
                        <MessageHeader label={label} timestamp={message.timestamp} />
                        <p className={kind.startsWith("tool_") ? styles.toolPayload : undefined}>
                          {preview.text}
                        </p>
                        <small>Click to open the full message</small>
                      </summary>
                      <p
                        className={`${styles.fullMessage} ${
                          kind.startsWith("tool_") ? styles.toolPayload : ""
                        }`}
                      >
                        {message.content}
                      </p>
                    </details>
                  ) : (
                    <>
                      <MessageHeader label={label} timestamp={message.timestamp} />
                      <p className={kind.startsWith("tool_") ? styles.toolPayload : undefined}>
                        {message.content}
                      </p>
                    </>
                  )}
                  {message.truncated ? <small>Only the stored preview is available</small> : null}
                </article>
              );
            })}
          </div>
        ) : (
          <EmptyState body="Conversation recording is off, or this session predates local redacted previews." />
        )}
      </article>
    </>
  );
}

function MessageHeader({ label, timestamp }: { label: string; timestamp: string }) {
  return (
    <header>
      <strong>{label}</strong>
      <span>{time(timestamp)}</span>
    </header>
  );
}

function messagePreview(content: string) {
  const maxCharacters = 280;
  const characters = [...content];
  if (characters.length <= maxCharacters) return { expandable: false, text: content };
  return { expandable: true, text: `${characters.slice(0, maxCharacters).join("")}…` };
}

function displayToolName(name?: string | null) {
  return name?.replaceAll("_", " ") || "tool";
}

// eslint-disable-next-line local/no-complex-business-logic -- Selects provider-specific visual marks only.
function ProviderMark({ detail = false, session }: { detail?: boolean; session: LocalSession }) {
  const family = providerFamily(session);
  const gradientId = useId();
  const label =
    family === "openai"
      ? "OpenAI"
      : family === "claude"
        ? "Claude"
        : family === "gemini"
          ? "Gemini"
          : displayProvider(session.provider);
  return (
    <span
      aria-label={`${label} provider`}
      className={`${styles.providerMark} ${styles[`provider${capitalize(family)}`]} ${
        detail ? styles.providerMarkDetail : ""
      }`}
      title={label}
    >
      {family === "openai" ? (
        <svg aria-hidden="true" viewBox="0 0 320 320">
          <image height="320" href="/logos/openai.svg" width="1180" />
        </svg>
      ) : null}
      {family === "claude" ? (
        <svg aria-hidden="true" viewBox="0 0 24 24">
          <g fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="2.4">
            <path d="M12 2.5v19M2.5 12h19M5.3 5.3l13.4 13.4M18.7 5.3 5.3 18.7" />
            <path d="m8.2 2.8 7.6 18.4M2.8 8.2l18.4 7.6M15.8 2.8 8.2 21.2M2.8 15.8l18.4-7.6" />
          </g>
        </svg>
      ) : null}
      {family === "gemini" ? (
        <svg aria-hidden="true" viewBox="0 0 24 24">
          <defs>
            <linearGradient id={gradientId} x1="3" x2="21" y1="3" y2="21">
              <stop stopColor="#4c8dff" />
              <stop offset="0.52" stopColor="#9b72f5" />
              <stop offset="1" stopColor="#ef72bd" />
            </linearGradient>
          </defs>
          <path
            d="M12 2c0 5.7-4.3 10-10 10 5.7 0 10 4.3 10 10 0-5.7 4.3-10 10-10-5.7 0-10-4.3-10-10Z"
            fill={`url(#${gradientId})`}
          />
        </svg>
      ) : null}
      {family === "unknown" ? <small>{initials(session.harness)}</small> : null}
    </span>
  );
}

function providerFamily(session: LocalSession) {
  const identity = `${session.provider} ${session.harness}`.toLowerCase();
  if (/openai|codex|chatgpt/.test(identity)) return "openai";
  if (/anthropic|claude/.test(identity)) return "claude";
  if (/gemini|google/.test(identity)) return "gemini";
  return "unknown";
}

function capitalize(value: string) {
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
}

function ContextView({
  context,
  organizationConnected,
  sessions,
  totalTokens,
}: {
  context: AttributedCategories;
  organizationConnected: boolean;
  sessions: LocalSession[];
  totalTokens: number;
}) {
  return (
    <section className={styles.contextPage}>
      <article className={styles.contextHero}>
        <p>Aggregate model usage</p>
        <strong>{compact(totalTokens)}</strong>
        <span>provider tokens across {sessions.length} sessions</span>
        <ContextStack context={context} />
        <ContextFindings context={context} scope="aggregate" />
      </article>
      <div className={styles.contextColumns}>
        <article className={styles.activityPanel}>
          <PanelHeader
            eyebrow="Provider-aligned attribution"
            meta="Normalized to input tokens"
            title="Composition"
          />
          <div className={styles.categoryGrid}>
            {CATEGORY_LABELS.map(([key, label]) => (
              <CategoryRow
                key={key}
                label={label}
                max={Math.max(...Object.values(context), 1)}
                value={context[key]}
              />
            ))}
          </div>
        </article>
        <article className={styles.explainerPanel}>
          <span>How to read this</span>
          <h2>Consumption is exact. Composition is attributed.</h2>
          <p>
            Cutokyo preserves provider token counters. It estimates each category from structured
            request fields, then normalizes that composition to provider-reported input tokens so
            failed attempts and retries without usage do not inflate consumption.
          </p>
          <ul>
            <li>
              <i />
              Provider total: authoritative
            </li>
            <li>
              <i />
              Context mix: estimated and normalized
            </li>
            <li>
              <i />
              {organizationConnected
                ? "Redacted conversation: organization governed"
                : "Redacted conversation: encrypted on this device"}
            </li>
          </ul>
        </article>
      </div>
    </section>
  );
}

function PluginsView({ snapshot }: { snapshot: DesktopSnapshot | null }) {
  const status = snapshot?.plugins;
  const active = status?.plugins.filter((plugin) => plugin.enabled && plugin.ready).length ?? 0;
  return (
    <section className={styles.pluginsPage}>
      <article className={styles.pluginHero}>
        <div>
          <span>Local extension runtime</span>
          <h2>{active} active plugins</h2>
          <p>
            Request plugins run after built-in redaction and before compression. Their telemetry is
            attached to the real operation event.
          </p>
        </div>
        <div>
          <strong>{status?.enabled ? "Runtime enabled" : "Runtime disabled"}</strong>
          <code>{status?.configPath ?? "Loading plugin configuration…"}</code>
        </div>
      </article>
      {status?.error ? (
        <div className={styles.errorBanner} role="alert">
          {status.error}
        </div>
      ) : null}
      <div className={styles.pluginGrid}>
        {status?.plugins.map((plugin) => (
          <article className={styles.pluginCard} key={plugin.id}>
            <header>
              <i
                className={plugin.enabled && plugin.ready ? styles.healthGood : styles.healthBad}
              />
              <strong>{plugin.id}</strong>
              <em>{plugin.enabled ? "enabled" : "disabled"}</em>
            </header>
            <p>{plugin.command || "No executable configured"}</p>
            <dl>
              <div>
                <dt>Hooks</dt>
                <dd>{plugin.hooks.join(", ") || "None"}</dd>
              </div>
              <div>
                <dt>Failure mode</dt>
                <dd>{plugin.failMode}</dd>
              </div>
              <div>
                <dt>Timeout</dt>
                <dd>{plugin.timeoutMs} ms</dd>
              </div>
            </dl>
          </article>
        ))}
        {status && !status.plugins.length ? (
          <EmptyState body="No plugins configured. Add real plugin definitions to the local configuration file shown above." />
        ) : null}
      </div>
    </section>
  );
}

function SettingsView({
  action,
  onAction,
  onUpdate,
  settings,
  snapshot,
}: {
  action: string | null;
  onAction: (action: "connect" | "deactivate") => Promise<void>;
  onUpdate: (settings: FeatureSettings) => Promise<void>;
  settings: FeatureSettings;
  snapshot: DesktopSnapshot | null;
}) {
  const organizationManaged = snapshot?.session.organizationConnected === true;
  const toggle = (key: keyof FeatureSettings) =>
    void onUpdate({ ...settings, [key]: !settings[key] });
  return (
    <section className={styles.settingsPage}>
      <article className={styles.connectionCard}>
        <div>
          <span>System detection</span>
          <h2>Local AI tools</h2>
          <p>
            Cutokyo configures supported harnesses and restores their original state when
            disconnected.
          </p>
        </div>
        <div className={styles.connectionActions}>
          <button
            className={styles.connectButton}
            disabled={snapshot === null || action !== null}
            onClick={() => void onAction("connect")}
            type="button"
          >
            {action === "connect" ? "Scanning…" : "Scan & connect"}
          </button>
          <button
            className={styles.dangerButton}
            disabled={snapshot === null || action !== null}
            onClick={() => void onAction("deactivate")}
            type="button"
          >
            Restore all
          </button>
        </div>
        <div className={styles.detectedTools}>
          {snapshot?.detection.map((client) => (
            <span key={client.id}>
              <i className={client.detected ? styles.healthGood : styles.healthBad} />
              {client.label}
              <em>{client.detected ? client.connection : "not detected"}</em>
            </span>
          ))}
          {!snapshot?.detection.length ? <EmptyState body="Scanning installed AI tools…" /> : null}
        </div>
      </article>
      <div className={styles.settingsGrid}>
        {SETTINGS.map((setting) => (
          <article key={setting.key}>
            <div>
              <strong>{setting.label}</strong>
              <p>{settingDetail(setting, snapshot)}</p>
              {organizationManaged && ORGANIZATION_MANAGED_SETTINGS.has(setting.key) ? (
                <small>Managed by your organization</small>
              ) : null}
            </div>
            <button
              aria-label={`${settings[setting.key] ? "Disable" : "Enable"} ${setting.label}`}
              aria-pressed={settings[setting.key]}
              className={styles.switch}
              disabled={
                snapshot === null ||
                action !== null ||
                (organizationManaged && ORGANIZATION_MANAGED_SETTINGS.has(setting.key))
              }
              onClick={() => toggle(setting.key)}
              type="button"
            >
              <i />
            </button>
          </article>
        ))}
      </div>
    </section>
  );
}

function settingDetail(setting: (typeof SETTINGS)[number], snapshot: DesktopSnapshot | null) {
  if (setting.key !== "conversationRecording") return setting.detail;
  if (snapshot?.session.organizationConnected) {
    const destination = snapshot.session.organizationName || "your organization";
    return `Store encrypted redacted previews and sync policy-approved copies to ${destination}.`;
  }
  return "Store encrypted redacted previews on this device.";
}

function ContextStack({ context }: { context: AttributedCategories }) {
  const total = Math.max(contextTotal(context), 1);
  return (
    <div aria-label="Context composition" className={styles.contextStack} role="img">
      {CATEGORY_LABELS.map(([key, label]) =>
        context[key] ? (
          <i
            key={key}
            style={
              {
                "--context-color": contextColor(key),
                "--context-width": `${(context[key] / total) * 100}%`,
              } as CSSProperties
            }
            title={`${label}: ${context[key]} tokens`}
          />
        ) : null,
      )}
    </div>
  );
}

function ContextFindings({
  context,
  scope,
}: {
  context: AttributedCategories;
  scope: "aggregate" | "session";
}) {
  const findings = contextFindings(context, scope);
  if (!findings.length) return null;
  return (
    <div aria-label={`${scope} context findings`} className={styles.contextFindings}>
      {findings.map((finding) => (
        <div key={finding.title}>
          <span>{finding.percent}%</span>
          <p>
            <strong>{finding.title}</strong>
            {finding.body}
          </p>
        </div>
      ))}
    </div>
  );
}

function OperationsTable({ events }: { events: OperationEvent[] }) {
  return (
    <div
      aria-label="Scrollable recent operations"
      className={styles.tableWrap}
      role="region"
      tabIndex={0}
    >
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Harness</th>
            <th>Provider / model</th>
            <th>Tokens</th>
            <th>Cache</th>
            <th>Cost</th>
            <th>Latency</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {events.map((event) => (
            <EventRow event={event} key={event.traceId} />
          ))}
        </tbody>
      </table>
      {!events.length ? (
        <EmptyState body="No operations yet. Your first model request will appear automatically." />
      ) : null}
    </div>
  );
}

function EventRow({ event }: { event: OperationEvent }) {
  return (
    <>
      <tr>
        <td>{time(event.timestamp)}</td>
        <td>{displayHarness(event.harness.name)}</td>
        <td>
          <strong>{displayProvider(event.provider)}</strong>
          <small>{event.responseModel ?? event.requestedModel ?? "Unknown model"}</small>
        </td>
        <td>{compact(event.usage.totalTokens ?? 0)}</td>
        <td>{compact(event.usage.cacheReadTokens ?? 0)}</td>
        <td>{usd(event.cost?.totalCostNanosUsd ?? 0)}</td>
        <td>
          {event.durationMs} ms
          <small>
            local {event.localProcessingMs ?? "—"} · provider {event.providerDurationMs ?? "—"} ·
            TTFB {event.timeToFirstByteMs ?? "—"} ms
          </small>
        </td>
        <td>
          <span className={event.outcome === "success" ? styles.success : styles.failure}>
            {event.statusCode}
          </span>
          <small>{event.errorType ?? event.outcome}</small>
        </td>
      </tr>
      <tr className={styles.operationDetailRow}>
        <td colSpan={8}>
          <details>
            <summary>Investigate {event.requestId ?? event.traceId}</summary>
            <dl className={styles.operationDetails}>
              <div>
                <dt>Trace ID</dt>
                <dd>{event.traceId}</dd>
              </div>
              <div>
                <dt>Request ID</dt>
                <dd>{event.requestId ?? "Unavailable"}</dd>
              </div>
              <div>
                <dt>Provider response ID</dt>
                <dd>{event.providerResponseId ?? "Unavailable"}</dd>
              </div>
              <div>
                <dt>Outcome / failure</dt>
                <dd>{event.errorType ? `${event.outcome} · ${event.errorType}` : event.outcome}</dd>
              </div>
              <div>
                <dt>API surface / transport</dt>
                <dd>
                  {event.apiSurface} · {event.transport}
                </dd>
              </div>
              <div>
                <dt>Preview state</dt>
                <dd>{event.contentRecorded ? "Policy-redacted preview" : "Metadata only"}</dd>
              </div>
              <div>
                <dt>Usage source</dt>
                <dd>{event.usageSource}</dd>
              </div>
              <div>
                <dt>Pricing</dt>
                <dd>
                  {event.cost?.pricingVersion ?? "Unavailable"} · {event.cost?.status ?? "unpriced"}
                </dd>
              </div>
              <div>
                <dt>Input / total input</dt>
                <dd>
                  {event.usage.inputTokens ?? 0} / {event.usage.totalInputTokens ?? 0}
                </dd>
              </div>
              <div>
                <dt>Output / reasoning</dt>
                <dd>
                  {event.usage.outputTokens ?? 0} / {event.usage.reasoningTokens ?? 0}
                </dd>
              </div>
              <div>
                <dt>Cache read / write</dt>
                <dd>
                  {event.usage.cacheReadTokens ?? 0} / {event.usage.cacheWriteTokens ?? 0}
                </dd>
              </div>
              <div>
                <dt>Total tokens</dt>
                <dd>{event.usage.totalTokens ?? 0}</dd>
              </div>
              <div>
                <dt>Optimization</dt>
                <dd>{JSON.stringify(event.optimization ?? {})}</dd>
              </div>
              <div>
                <dt>Security</dt>
                <dd>{JSON.stringify(event.security ?? {})}</dd>
              </div>
              <div>
                <dt>Plugins</dt>
                <dd>{JSON.stringify(event.plugins ?? {})}</dd>
              </div>
            </dl>
            {event.contentRecorded && event.conversation?.length ? (
              <div className={styles.operationPreview}>
                <strong>Policy-redacted content</strong>
                {event.conversation.map((message, index) => (
                  <p key={`${message.role}-${index}`}>
                    <b>{message.role}</b> {message.content}
                    {message.truncated ? " …" : ""}
                  </p>
                ))}
              </div>
            ) : null}
          </details>
        </td>
      </tr>
    </>
  );
}

function MetricCard({
  accent = false,
  label,
  note,
  tokenValue = false,
  value,
}: {
  accent?: boolean;
  label: string;
  note: string;
  tokenValue?: boolean;
  value: string;
}) {
  return (
    <article className={accent ? styles.metricAccent : styles.metric}>
      <span>{label}</span>
      <strong className={tokenValue ? styles.tokenValue : undefined}>{value}</strong>
      <small>{note}</small>
    </article>
  );
}

function PanelHeader({ eyebrow, meta, title }: { eyebrow: string; meta: string; title: string }) {
  return (
    <header className={styles.panelHeader}>
      <div>
        <span>{eyebrow}</span>
        <h2>{title}</h2>
      </div>
      <em>{meta}</em>
    </header>
  );
}

function CategoryRow({ label, max, value }: { label: string; max: number; value: number }) {
  return (
    <div className={styles.categoryRow}>
      <div>
        <span>{label}</span>
        <strong>{compact(value)}</strong>
      </div>
      <i>
        <b
          style={
            {
              "--category-width": `${max ? Math.max(1, (value / max) * 100) : 0}%`,
            } as CSSProperties
          }
        />
      </i>
    </div>
  );
}

function HealthCard({
  detail,
  label,
  status,
  tone,
}: {
  detail: string;
  label: string;
  status: string;
  tone: "bad" | "good" | "neutral";
}) {
  return (
    <article className={styles.healthCard}>
      <span>{label}</span>
      <strong>
        <i
          className={
            tone === "good"
              ? styles.healthGood
              : tone === "bad"
                ? styles.healthBad
                : styles.healthNeutral
          }
        />
        {status}
      </strong>
      <p>{detail}</p>
    </article>
  );
}

function EmptyState({ body }: { body: string }) {
  return <p className={styles.empty}>{body}</p>;
}

function topContext(context: AttributedCategories, limit: number) {
  return (Object.entries(context) as [keyof AttributedCategories, number][])
    .sort((left, right) => right[1] - left[1])
    .slice(0, limit);
}

function contextTotal(context: AttributedCategories) {
  return Object.values(context).reduce((total, value) => total + value, 0);
}

function contextFindings(context: AttributedCategories, scope: "aggregate" | "session") {
  const total = contextTotal(context);
  if (!total) return [];
  const combined = [
    {
      body: "MCP definitions and results occupy model context.",
      title: "MCP context",
      value: context.mcpDefinitions + context.mcpResults,
    },
    {
      body: "Tool schemas and results are the largest optimization candidates.",
      title: "Tool context",
      value: context.toolDefinitions + context.toolResults,
    },
    {
      body: "Conversation history grows with each turn and may benefit from a fresh session.",
      title: "Conversation history",
      value: context.conversationHistory,
    },
  ];
  return combined
    .map((finding) => ({ ...finding, percent: Math.round((finding.value / total) * 100) }))
    .filter((finding) => finding.value > 0)
    .sort((left, right) => right.value - left.value)
    .slice(0, 3);
}

function categoryLabel(key: keyof AttributedCategories) {
  return CATEGORY_LABELS.find(([candidate]) => candidate === key)?.[1] ?? key;
}

function contextColor(key: keyof AttributedCategories) {
  const colors: Record<keyof AttributedCategories, string> = {
    conversationHistory: "#c3f400",
    currentUserInput: "#f5f4ea",
    instructions: "#32b7ff",
    mcpDefinitions: "#ff8a5b",
    mcpResults: "#ffbf3f",
    media: "#bd83ff",
    otherContext: "#60645c",
    retrievedDocuments: "#4ed9a3",
    toolDefinitions: "#f264d6",
    toolResults: "#ff6577",
  };
  return colors[key];
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
function percent(ratio: number) {
  return `${Math.round(ratio * 100)}%`;
}
function time(timestamp: string) {
  return new Intl.DateTimeFormat("en", { hour: "2-digit", minute: "2-digit" }).format(
    new Date(timestamp),
  );
}
// eslint-disable-next-line local/no-complex-business-logic -- Formats a timestamp into a compact display label only.
function relativeTime(timestamp: string) {
  const minutes = Math.max(0, Math.round((Date.now() - new Date(timestamp).getTime()) / 60_000));
  return minutes < 1
    ? "now"
    : minutes < 60
      ? `${minutes}m ago`
      : minutes < 1_440
        ? `${Math.floor(minutes / 60)}h ago`
        : `${Math.floor(minutes / 1_440)}d ago`;
}
function initials(name: string) {
  return name
    .split(/[-_ ]/)
    .map((part) => part[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}
function displayHarness(name: string) {
  return (
    {
      antigravity: "Antigravity",
      "chatgpt-app": "ChatGPT app",
      "claude-code": "Claude Code",
      "claude-desktop": "Claude Desktop",
      "codex-app": "Codex Desktop",
      "codex-cli": "Codex CLI",
      "gemini-cli": "Gemini CLI",
      sdk: "SDK",
      unknown: "Unknown",
    }[name] ?? name
  );
}
function displayProvider(name: string) {
  if (name.startsWith("openai") || name.startsWith("codex")) return "OpenAI";
  if (name.startsWith("anthropic") || name.startsWith("claude")) return "Anthropic";
  if (name.startsWith("gemini")) return "Google";
  return name;
}

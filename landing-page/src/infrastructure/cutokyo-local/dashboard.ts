"use client";

import { useCallback, useEffect, useMemo, useState } from "react";

import { isCutokyoAutostartEnabled, syncCutokyoAutostart } from "@/infrastructure/tauri/autostart";
import { openExternalUrl } from "@/infrastructure/tauri/open-url";
import { env } from "@/shared/config/env";
import { MAX_JSON_RESPONSE_BYTES, readBoundedJson } from "@/shared/http/bounded-json";

const LOCAL_PROXY_URL = env.NEXT_PUBLIC_CUTOKYO_LOCAL_API_URL;
export const MAX_LOCAL_RESPONSE_BYTES = MAX_JSON_RESPONSE_BYTES;

export type ProviderUsage = {
  cacheReadTokens?: number | null;
  cacheWriteTokens?: number | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  reasoningTokens?: number | null;
  totalInputTokens?: number | null;
  totalTokens?: number | null;
};

export type AttributedCategories = {
  conversationHistory: number;
  currentUserInput: number;
  instructions: number;
  mcpDefinitions: number;
  mcpResults: number;
  media: number;
  otherContext: number;
  retrievedDocuments: number;
  toolDefinitions: number;
  toolResults: number;
};

export type FeatureSettings = {
  apiKeyRedaction: boolean;
  autoConnect: boolean;
  compression: boolean;
  conversationRecording: boolean;
  emailRedaction: boolean;
  filePathRedaction: boolean;
  ipAddressRedaction: boolean;
  observability: boolean;
  organizationSync: boolean;
  otlpExport: boolean;
  paymentCardRedaction: boolean;
  phoneNumberRedaction: boolean;
  plugins: boolean;
  providerMutation: boolean;
};

export type LocalSession = {
  cachedTokens: number;
  context: AttributedCategories;
  conversationId?: string | null;
  errors: number;
  estimatedCostNanosUsd: number;
  harness: string;
  id: string;
  inputTokens: number;
  lastActivityAt: string;
  models: string[];
  operations: number;
  outputTokens: number;
  project?: string | null;
  provider: string;
  reasoningTokens: number;
  startedAt: string;
  title: string;
  totalTokens: number;
  traceIds: string[];
  messages: {
    callId?: string | null;
    content: string;
    kind?: string;
    name?: string | null;
    role: string;
    timestamp: string;
    traceId: string;
    truncated: boolean;
  }[];
  workspace?: string | null;
};

export type OperationEvent = {
  apiSurface: string;
  attributedUsage: {
    categories: AttributedCategories;
    measurement: string;
    method: string;
    totalEstimatedTokens: number;
  };
  contentRecorded: boolean;
  errorType?: string | null;
  conversation?: {
    callId?: string | null;
    content: string;
    kind?: string;
    name?: string | null;
    role: string;
    truncated: boolean;
  }[];
  cost?: {
    canonicalModel?: string | null;
    currency: string;
    estimated: boolean;
    pricingVersion: string;
    status: string;
    totalCostNanosUsd?: number | null;
    totalCostUsd?: string | null;
    unavailableReason?: string | null;
  } | null;
  conversationId?: string | null;
  durationMs: number;
  localProcessingMs?: number | null;
  harness: {
    confidence: number;
    method: string;
    name: string;
  };
  optimization?: Record<string, unknown>;
  outcome: string;
  plugins?: Record<string, unknown>;
  project?: string | null;
  provider: string;
  providerDurationMs?: number | null;
  providerResponseId?: string | null;
  requestId?: string | null;
  requestedModel?: string | null;
  responseModel?: string | null;
  statusCode: number;
  security?: Record<string, unknown> & {
    redacted?: boolean;
    redactionCount?: number;
  };
  timeToFirstByteMs?: number | null;
  timestamp: string;
  traceId: string;
  transport: string;
  usage: ProviderUsage;
  usageSource: string;
  workspace?: string | null;
};

type OperationEventPage = {
  generation: string | null;
  nextCursor: number | null;
  operations: OperationEvent[];
};

export type DesktopSnapshot = {
  connectivity?: {
    probedAt?: string | null;
    providers: {
      error?: string | null;
      latencyMs: number;
      provider: string;
      reachable: boolean;
      statusCode?: number | null;
    }[];
    reachable?: boolean | null;
  };
  detection: {
    connection: string;
    detected: boolean;
    evidence: string[];
    id: string;
    kind: string;
    label: string;
  }[];
  eventGeneration?: string | null;
  events: OperationEvent[];
  eventsNextCursor?: number | null;
  settings: FeatureSettings;
  sessions: LocalSession[];
  exporter: {
    enabled: boolean;
    endpoint?: string | null;
    exportedEvents: number;
    lastError?: string | null;
    lastSuccess?: string | null;
    pendingEvents: number;
    protocol: string;
  };
  metrics: {
    compressedRequests: number;
    liveRequests: number;
    savingsRatio: number;
    totalEstimatedTokensAfter: number;
    totalEstimatedTokensBefore: number;
    totalEstimatedTokensSaved: number;
  };
  organization?: {
    configured: boolean;
    connected: boolean;
    enabled: boolean;
    deviceId?: string | null;
    lastAttempt?: string | null;
    lastError?: string | null;
    lastSuccess?: string | null;
    pendingEvents: number;
    uploadedEvents: number;
  };
  plugins: {
    configured: boolean;
    configPath: string;
    enabled: boolean;
    error?: string | null;
    plugins: {
      command: string;
      enabled: boolean;
      failMode: string;
      hooks: string[];
      id: string;
      ready: boolean;
      timeoutMs: number;
    }[];
  };
  storage: {
    blockedRecords: number;
    degraded: boolean;
    encryption: string;
    keySource: string;
    lastWriteAt?: string | null;
    lastWriteError?: string | null;
    quarantinedRecords: number;
    records: number;
    retentionError?: string | null;
  };
  session: {
    authConfigured: boolean;
    authenticated: boolean;
    connectionError?: string | null;
    controlPlaneConfigured?: boolean;
    deviceId?: string | null;
    organizationConnected?: boolean;
    organizationId?: string | null;
    organizationName?: string | null;
    storageError?: string | null;
    user?: {
      email?: string | null;
      name?: string | null;
      subject?: string | null;
    } | null;
  };
  status: {
    clients: Record<string, string>;
    status: string;
  };
};

export const CATEGORY_LABELS: [keyof AttributedCategories, string][] = [
  ["conversationHistory", "Conversation history"],
  ["toolDefinitions", "Tool definitions"],
  ["toolResults", "Tool results"],
  ["mcpDefinitions", "MCP definitions"],
  ["mcpResults", "MCP results"],
  ["instructions", "System + developer"],
  ["currentUserInput", "Current user input"],
  ["retrievedDocuments", "Retrieved documents"],
  ["media", "Images + media"],
  ["otherContext", "Other context"],
];

const EMPTY_CATEGORIES: AttributedCategories = {
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

type DashboardAction = "connect" | "deactivate" | "signin" | "signout";

export function useLocalDesktopDashboard(initialData?: DesktopSnapshot) {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(initialData ?? null);
  const [error, setError] = useState<string | null>(null);
  const [action, setAction] = useState<DashboardAction | null>(null);
  const [operationBrowsePinned, setOperationBrowsePinned] = useState(false);
  const [operationCorrelation, setOperationCorrelation] = useState("");
  const summary = useMemo(() => summarize(snapshot), [snapshot]);
  const visibleError =
    error ??
    snapshot?.session.storageError ??
    snapshot?.session.connectionError ??
    snapshot?.organization?.lastError ??
    null;
  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      try {
        const next = await fetchDesktopSnapshot(signal);
        setSnapshot((current) =>
          operationBrowsePinned && current
            ? {
                ...next,
                eventGeneration: current.eventGeneration ?? null,
                events: current.events,
                eventsNextCursor: current.eventsNextCursor ?? null,
              }
            : next,
        );
        setError(null);
      } catch (refreshError) {
        if (refreshError instanceof DOMException && refreshError.name === "AbortError") return;
        setError(dashboardErrorMessage(refreshError, "Local proxy unavailable"));
      }
    },
    [operationBrowsePinned],
  );

  useEffect(() => {
    if (process.env.NODE_ENV === "test") return;
    let active = true;
    let controller: AbortController | null = null;
    let timer: number | null = null;
    const poll = async () => {
      controller = new AbortController();
      await refresh(controller.signal);
      if (active) timer = window.setTimeout(() => void poll(), 2500);
    };
    void poll();
    return () => {
      active = false;
      controller?.abort();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [refresh]);

  const runAction = useCallback(
    async (nextAction: DashboardAction) => {
      setAction(nextAction);
      setError(null);
      try {
        await dashboardAction(nextAction);
        await refresh();
      } catch (actionError) {
        setError(dashboardErrorMessage(actionError, "Action failed"));
      } finally {
        setAction(null);
      }
    },
    [refresh],
  );

  const updateSettings = useCallback(async (settings: FeatureSettings) => {
    setError(null);
    try {
      const saved = await request<FeatureSettings>("/cutokyo/settings", undefined, {
        body: JSON.stringify(settings),
        headers: { "content-type": "application/json" },
        method: "PUT",
      });
      setSnapshot((current) => (current ? { ...current, settings: saved } : current));
    } catch (settingsError) {
      setError(dashboardErrorMessage(settingsError, "Settings update failed"));
    }
  }, []);

  const loadOperations = useCallback(async (correlation: string, cursor?: number) => {
    setError(null);
    try {
      const normalized = correlation.trim();
      const page = await fetchOperationPage(normalized || undefined, cursor);
      setOperationBrowsePinned(Boolean(normalized || cursor !== undefined));
      setOperationCorrelation(normalized);
      setSnapshot((current) =>
        current
          ? {
              ...current,
              eventGeneration: page.generation,
              events:
                cursor === undefined ||
                (current.eventGeneration !== undefined &&
                  current.eventGeneration !== page.generation)
                  ? page.operations
                  : [...current.events, ...page.operations],
              eventsNextCursor: page.nextCursor,
            }
          : current,
      );
    } catch (operationError) {
      setError(dashboardErrorMessage(operationError, "Operations unavailable"));
    }
  }, []);

  return {
    action,
    error: visibleError,
    loadOperations,
    operationCorrelation,
    refresh,
    runAction,
    snapshot,
    summary,
    updateSettings,
  };
}

async function dashboardAction(action: DashboardAction) {
  if (action === "connect") return connectDesktopClients();
  if (action === "deactivate") return deactivateDesktopClients();
  if (action === "signin") return startDesktopSignIn();
  return signOutDesktop();
}

async function fetchDesktopSnapshot(signal?: AbortSignal): Promise<DesktopSnapshot> {
  const [
    status,
    connectivity,
    detection,
    events,
    exporter,
    metrics,
    organization,
    plugins,
    sessions,
    session,
    settings,
    storage,
  ] = await Promise.all([
    request<DesktopSnapshot["status"]>("/cutokyo/status", signal),
    request<NonNullable<DesktopSnapshot["connectivity"]>>("/cutokyo/connectivity/probe", signal),
    request<DesktopSnapshot["detection"]>("/cutokyo/detection", signal),
    fetchOperationPage(undefined, undefined, signal),
    request<DesktopSnapshot["exporter"]>("/cutokyo/export/status", signal),
    request<DesktopSnapshot["metrics"]>("/cutokyo/metrics", signal),
    request<NonNullable<DesktopSnapshot["organization"]>>("/cutokyo/organization/status", signal),
    request<DesktopSnapshot["plugins"]>("/cutokyo/plugins/status", signal),
    request<LocalSession[]>("/cutokyo/sessions", signal),
    request<DesktopSnapshot["session"]>("/cutokyo/auth/session", signal),
    request<FeatureSettings>("/cutokyo/settings", signal),
    request<DesktopSnapshot["storage"]>("/cutokyo/events/status", signal),
  ]);
  return {
    connectivity,
    detection,
    eventGeneration: events.generation,
    events: events.operations,
    eventsNextCursor: events.nextCursor,
    exporter,
    metrics,
    organization,
    plugins,
    sessions,
    session,
    settings,
    status,
    storage,
  };
}

async function connectDesktopClients(): Promise<void> {
  const autostartWasEnabled = await isCutokyoAutostartEnabled();
  await syncCutokyoAutostart(true);
  try {
    await request("/cutokyo/activation/auto", undefined, { method: "POST" });
  } catch (error) {
    if (!autostartWasEnabled) {
      await syncCutokyoAutostart(false).catch(() => undefined);
    }
    throw error;
  }
  await request("/cutokyo/connectivity/probe", undefined, { method: "POST" });
}

async function deactivateDesktopClients(): Promise<void> {
  await request("/cutokyo/activation/auto", undefined, { method: "DELETE" });
  await syncCutokyoAutostart(false);
}

async function startDesktopSignIn(): Promise<void> {
  const response = await request<{ authorizationUrl: string }>("/cutokyo/auth/start", undefined, {
    method: "POST",
  });
  await openExternalUrl(response.authorizationUrl);
}

async function signOutDesktop(): Promise<void> {
  await request("/cutokyo/auth/sign-out", undefined, { method: "POST" });
}

async function fetchOperationPage(
  correlationId?: string,
  cursor?: number,
  signal?: AbortSignal,
): Promise<OperationEventPage> {
  const query = new URLSearchParams();
  if (correlationId) query.set("correlationId", correlationId);
  if (cursor !== undefined) query.set("cursor", String(cursor));
  query.set("limit", "50");
  return request<OperationEventPage>(`/cutokyo/events/page?${query.toString()}`, signal);
}

async function request<T>(path: string, signal?: AbortSignal, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  headers.set("accept", "application/json");
  const requestInit: RequestInit = { ...init, headers };
  if (signal) requestInit.signal = signal;
  const response = await fetch(`${LOCAL_PROXY_URL}${path}`, requestInit);
  if (!response.ok) {
    const body = await readBoundedJson<{ detail?: unknown } | null>(
      response,
      "Local proxy response",
    ).catch(() => null);
    const detail = typeof body?.detail === "string" ? body.detail : `HTTP ${response.status}`;
    throw new Error(detail);
  }
  return readBoundedJson<T>(response, "Local proxy response");
}

export function dashboardErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof TypeError) {
    return `Cutokyo proxy is unavailable at ${new URL(LOCAL_PROXY_URL).host}.`;
  }
  return error instanceof Error ? error.message : fallback;
}

function summarize(snapshot: DesktopSnapshot | null) {
  const categories = { ...EMPTY_CATEGORIES };
  const harnesses = new Map<string, { name: string; operations: number; tokens: number }>();
  const sessions = snapshot?.sessions ?? [];
  let totalTokens = 0;
  let cachedTokens = 0;
  let costNanos = 0;
  let attributedTotal = 0;
  let pricingVersion: string | null = null;
  let redactions = 0;
  for (const session of sessions) {
    totalTokens += session.totalTokens;
    cachedTokens += session.cachedTokens;
    costNanos += session.estimatedCostNanosUsd;
    attributedTotal += session.inputTokens;
    addCategories(categories, providerAlignedContext(session.context, session.inputTokens));
  }
  for (const event of snapshot?.events ?? []) {
    if (!sessions.length) {
      totalTokens += event.usage.totalTokens ?? 0;
      cachedTokens += event.usage.cacheReadTokens ?? 0;
      costNanos += event.cost?.totalCostNanosUsd ?? 0;
      attributedTotal += event.usage.totalInputTokens ?? event.usage.inputTokens ?? 0;
      addCategories(
        categories,
        providerAlignedContext(
          event.attributedUsage.categories,
          event.usage.totalInputTokens ?? event.usage.inputTokens ?? 0,
        ),
      );
    }
    pricingVersion ??= event.cost?.pricingVersion ?? null;
    redactions += event.security?.redactionCount ?? 0;
    addHarness(harnesses, event);
  }
  return {
    attributedTotal,
    cachedTokens,
    categories,
    costNanos,
    events: snapshot?.events.length ?? 0,
    harnesses: [...harnesses.values()].sort((left, right) => right.tokens - left.tokens),
    maxCategory: Math.max(...Object.values(categories), 0),
    pricingVersion,
    redactions,
    totalTokens,
  };
}

export function providerAlignedContext(
  context: AttributedCategories,
  providerInputTokens: number,
): AttributedCategories {
  const aligned = { ...EMPTY_CATEGORIES };
  const target = Math.max(0, Math.round(providerInputTokens));
  if (!target) return aligned;
  const estimatedTotal = Object.values(context).reduce((total, value) => total + value, 0);
  if (!estimatedTotal) return { ...aligned, otherContext: target };
  for (const key of Object.keys(aligned) as (keyof AttributedCategories)[]) {
    aligned[key] = Math.floor((context[key] * target) / estimatedTotal);
  }
  const allocated = Object.values(aligned).reduce((total, value) => total + value, 0);
  aligned.otherContext += target - allocated;
  return aligned;
}

function addCategories(target: AttributedCategories, source: AttributedCategories) {
  for (const key of Object.keys(target) as (keyof AttributedCategories)[]) {
    target[key] += source[key];
  }
}

function addHarness(
  harnesses: Map<string, { name: string; operations: number; tokens: number }>,
  event: OperationEvent,
) {
  const harness = harnesses.get(event.harness.name) ?? {
    name: event.harness.name,
    operations: 0,
    tokens: 0,
  };
  harness.operations += 1;
  harness.tokens += event.usage.totalTokens ?? 0;
  harnesses.set(event.harness.name, harness);
}

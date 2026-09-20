import type {
  AnalysisResult,
  Coverage,
  DashboardResponse,
  DesktopSettings,
  GuardsResponse,
  HealthResponse,
  InventoryResponse,
  OnboardingResponse,
  Provenance,
  SessionRecord,
} from "../contracts.js";

export const FIXTURE_NOW = "2026-09-19T16:42:00Z";

const completeCoverage: Coverage = {
  state: "complete",
  scope: "Synthetic native event and local state fixture",
  gaps: [],
};

const partialCoverage: Coverage = {
  state: "partial",
  scope: "Synthetic native session metadata",
  gaps: ["Tool results before native history page 2 were not exposed"],
};

const unavailableCoverage: Coverage = {
  state: "unavailable",
  scope: "Provider-bound request context",
  gaps: ["Proxy capture is disabled"],
};

function provenance(
  harness: "claude_code" | "codex" | "opencode",
  channel = "hook_or_plugin",
  coverage: Coverage = completeCoverage,
): Provenance {
  return {
    channel,
    sourceTier:
      channel === "hook_or_plugin" ? 1 : channel === "open_telemetry" ? 3 : 5,
    capturedAt: FIXTURE_NOW,
    parserVersion: "fixture-parser-1",
    confidence: coverage.state === "partial" ? "estimated" : "observed",
    coverage,
    observationIds: [`obs:${harness}:73A9`],
  };
}

function usage(
  harness: "claude_code" | "codex" | "opencode",
  nativeUsageKey: string,
  inputTokens: number,
  outputTokens: number,
  cacheReadTokens: number | null,
) {
  return {
    nativeUsageKey,
    harness,
    model:
      harness === "claude_code"
        ? "claude-sonnet-4-5"
        : harness === "codex"
          ? "gpt-5-codex"
          : "provider-model-unreported",
    inputTokens,
    outputTokens,
    cacheReadTokens,
    cacheWriteTokens: null,
    providerCostMicros: null,
    billingBasis:
      harness === "opencode"
        ? ("subscription" as const)
        : ("token_price" as const),
    provenance: provenance(harness, "open_telemetry"),
  };
}

export const dashboardSessions: readonly SessionRecord[] = [
  {
    id: "session-claude-73A9",
    harness: "claude_code",
    nativeSessionKey: "claude-tree-73A9",
    nativeResumeId: "claude-jev-73A9",
    title: "Reconcile usage capture",
    summary:
      "JEV exact resume needle 73A9 · reconciled native usage without proxy double counting.",
    project: "cutokyo-community",
    branch: "feature/observability",
    startedAt: "2026-09-19T14:08:00Z",
    endedAt: "2026-09-19T14:46:00Z",
    state: "completed",
    tools: ["Read", "Bash"],
    skills: ["cutokyo-contract"],
    agents: ["desktop-builder"],
    usage: [
      usage("claude_code", "request-claude-73A9", 64_200, 16_800, 32_000),
    ],
    timeline: [
      {
        id: "timeline-c1",
        kind: "user",
        at: "2026-09-19T14:08:00Z",
        title: "Inspect reconciliation mismatch",
        body: "Compare native OTel usage with the overlapping proxy observation.",
        state: "succeeded",
        provenance: provenance("claude_code"),
      },
      {
        id: "timeline-c2",
        kind: "tool",
        at: "2026-09-19T14:11:20Z",
        title: "Read · usage projection",
        body: "Native request key request-claude-73A9 selected by source precedence.",
        state: "succeeded",
        provenance: provenance("claude_code"),
      },
      {
        id: "timeline-c3",
        kind: "assistant",
        at: "2026-09-19T14:46:00Z",
        title: "Reconciliation complete",
        body: "The proxy observation remains linked evidence and is not added twice.",
        state: "succeeded",
        provenance: provenance("claude_code", "open_telemetry"),
      },
    ],
    provenance: provenance("claude_code"),
  },
  {
    id: "session-codex-91B2",
    harness: "codex",
    nativeSessionKey: "codex-session-tree-91B2",
    nativeResumeId: "thread-codex-91B2",
    title: "Verify migration boundaries",
    summary:
      "Checked schema downgrade refusal and retained an incomplete-history warning.",
    project: "cutokyo-community",
    branch: "feature/storage",
    startedAt: "2026-09-18T10:12:00Z",
    endedAt: "2026-09-18T11:03:00Z",
    state: "completed",
    tools: ["cargo test"],
    skills: [],
    agents: ["core-builder"],
    usage: [usage("codex", "request-codex-91B2", 78_000, 17_510, 0)],
    timeline: [
      {
        id: "timeline-x1",
        kind: "system",
        at: "2026-09-18T10:12:00Z",
        title: "History coverage is partial",
        body: "The app server reported another page but did not provide its cursor.",
        state: "unknown",
        provenance: provenance("codex", "local_api", partialCoverage),
      },
    ],
    provenance: provenance("codex", "local_api", partialCoverage),
  },
  {
    id: "session-opencode-42C7",
    harness: "opencode",
    nativeSessionKey: "opencode-session-42C7",
    nativeResumeId: "opencode-session-42C7",
    title: "Trace plugin finalization",
    summary: "Observed the final tool result after the session idle event.",
    project: "fixture-plugin-lab",
    branch: "main",
    startedAt: "2026-09-17T08:40:00Z",
    endedAt: "2026-09-17T09:05:00Z",
    state: "completed",
    tools: ["plugin.verify"],
    skills: ["protocol-check"],
    agents: [],
    usage: [usage("opencode", "request-opencode-42C7", 42_000, 8_500, null)],
    timeline: [
      {
        id: "timeline-o1",
        kind: "tool",
        at: "2026-09-17T09:04:58Z",
        title: "plugin.verify",
        body: "Final output received before projection completion.",
        state: "succeeded",
        provenance: provenance("opencode"),
      },
    ],
    provenance: provenance("opencode"),
  },
];

const analysisSession: SessionRecord = {
  ...dashboardSessions[0]!,
  id: "analysis-73A9",
  nativeSessionKey: "analysis-tree-73A9",
  nativeResumeId: "analysis-native-73A9",
  title: "analysis-73A9",
  summary: "A fixture session selected for consented analysis.",
  usage: [],
};

const deleteSession: SessionRecord = {
  ...dashboardSessions[1]!,
  id: "delete-me-73A9",
  nativeSessionKey: "delete-me-73A9",
  nativeResumeId: "thread-delete-me-73A9",
  title: "delete-me-73A9",
  startedAt: "2026-06-01T10:00:00Z",
  usage: [],
};

const keepSession: SessionRecord = {
  ...dashboardSessions[2]!,
  id: "keep-me-73A9",
  nativeSessionKey: "keep-me-73A9",
  nativeResumeId: "keep-me-73A9",
  title: "keep-me-73A9",
  startedAt: "2026-09-18T10:00:00Z",
  usage: [],
};

const baseOnboarding: OnboardingResponse = {
  meta: {
    freshness: "partial",
    notices: [
      "OpenCode is installed but its capture plugin is not configured.",
    ],
    generatedAt: FIXTURE_NOW,
  },
  complete: true,
  storageDisclosure:
    "History is stored locally in a plaintext SQLite database with owner-only permissions. Cutokyo v0.x is not application-encrypted; full-disk encryption is recommended.",
  telemetryDisclosure:
    "Telemetry is off. Proxy capture and AI analysis are separate optional egress paths and remain disabled until you explicitly consent.",
  harnesses: [
    {
      harness: "claude_code",
      displayName: "Claude Code",
      capture: completeCoverage,
      inventory: completeCoverage,
      transcript: completeCoverage,
      resume: completeCoverage,
      source: "Native hooks + OpenTelemetry",
      lastSeenAt: "2026-09-19T14:46:00Z",
      actionable: null,
    },
    {
      harness: "codex",
      displayName: "Codex",
      capture: partialCoverage,
      inventory: completeCoverage,
      transcript: partialCoverage,
      resume: completeCoverage,
      source: "App server + local state",
      lastSeenAt: "2026-09-18T11:03:00Z",
      actionable: "Refresh app-server history to attempt the missing page.",
    },
    {
      harness: "opencode",
      displayName: "OpenCode",
      capture: {
        state: "disabled",
        scope: "OpenCode plugin events",
        gaps: ["Capture plugin is not installed"],
      },
      inventory: completeCoverage,
      transcript: unavailableCoverage,
      resume: completeCoverage,
      source: "Server API (inventory only)",
      lastSeenAt: "2026-09-17T09:05:00Z",
      actionable: "Install the Cutokyo OpenCode capture plugin.",
    },
  ],
};

const baseDashboard: DashboardResponse = {
  meta: {
    freshness: "partial",
    notices: [
      "Codex history is partial; totals include only attributable observed usage.",
    ],
    generatedAt: FIXTURE_NOW,
  },
  sessions: dashboardSessions,
  prices: [
    {
      provider: "Anthropic",
      model: "claude-sonnet-4-5",
      currency: "USD",
      inputMicrosPerMillion: 3_000_000,
      outputMicrosPerMillion: 15_000_000,
      validFrom: "2026-09-01T00:00:00Z",
      validUntil: "2026-10-01T00:00:00Z",
      confidence: "estimated",
      label:
        "Estimate from a validity-bounded local price snapshot; not a provider invoice.",
      provenance: provenance("claude_code", "provider_usage_api"),
    },
    {
      provider: "Operator declaration",
      model: "gpt-5-codex",
      currency: "USD",
      inputMicrosPerMillion: null,
      outputMicrosPerMillion: null,
      validFrom: "2026-09-01T00:00:00Z",
      validUntil: null,
      confidence: "user_declared",
      label: "User-declared subscription; no per-request charge is claimed.",
      provenance: {
        ...provenance("codex", "user_declaration"),
        confidence: "user_declared",
      },
    },
  ],
  quotas: [
    {
      name: "Claude 5-hour window",
      limit: 200,
      used: 128,
      remaining: 72,
      resetsAt: "2026-09-19T19:00:00Z",
      confidence: "estimated",
      label:
        "Learned ceiling from local headers; not an authoritative account quota.",
      provenance: {
        ...provenance("claude_code", "local_state"),
        confidence: "estimated",
      },
    },
    {
      name: "Codex account quota",
      limit: null,
      used: null,
      remaining: null,
      resetsAt: null,
      confidence: "unknown",
      label: "The current source does not expose a quota. Unknown is not zero.",
      provenance: {
        ...provenance("codex", "local_api", unavailableCoverage),
        confidence: "unknown",
      },
    },
  ],
  contextBreakdown: null,
  conflicts: [
    {
      fact: "request-claude-73A9 token usage",
      winner: "OpenTelemetry · source tier 3",
      ignored: "Consented proxy observation · source tier 8",
      reason:
        "Both observations carry the same native request key, so the lower-priority source remains evidence but is not added again.",
    },
  ],
  captureLive: true,
};

const baseInventory: InventoryResponse = {
  meta: {
    freshness: "partial",
    notices: [
      "One project plugin is degraded; other broker routes remain available.",
    ],
    generatedAt: FIXTURE_NOW,
  },
  brokerState: "degraded",
  searchMcpEnabled: true,
  items: [
    {
      id: "mcp-docs-73A9",
      kind: "mcp",
      name: "docs-73A9",
      harnesses: ["claude_code", "codex", "opencode"],
      scope: "managed",
      origin: "Cutokyo central broker · managed upstream",
      state: "enabled",
      managedByCutokyo: true,
      description:
        "Namespaced documentation tools routed consistently to all three harnesses.",
      provenance: provenance("claude_code", "local_state"),
    },
    {
      id: "mcp-cutokyo-search",
      kind: "mcp",
      name: "Cutokyo search MCP",
      harnesses: ["claude_code", "codex", "opencode"],
      scope: "managed",
      origin: "Cutokyo read-only agent surface",
      state: "enabled",
      managedByCutokyo: true,
      description:
        "Read-only session and coverage search. It never injects context automatically.",
      provenance: provenance("claude_code", "local_state"),
    },
    {
      id: "plugin-fixture-73A9",
      kind: "plugin",
      name: "fixture-processor-73A9",
      harnesses: ["claude_code"],
      scope: "project",
      origin: "Project plugin manifest · synthetic fixture",
      state: "enabled",
      managedByCutokyo: false,
      description:
        "Protocol v1 processor with transcript access denied and network access denied.",
      provenance: provenance("claude_code", "local_state"),
    },
    {
      id: "plugin-stale",
      kind: "plugin",
      name: "release-notes-indexer",
      harnesses: ["opencode"],
      scope: "project",
      origin: "Project plugin manifest",
      state: "degraded",
      managedByCutokyo: false,
      description:
        "Handshake timed out. Other plugins and MCP routes continue independently.",
      provenance: provenance("opencode", "local_state", partialCoverage),
    },
    {
      id: "hook-session-start",
      kind: "hook",
      name: "Cutokyo SessionStart",
      harnesses: ["claude_code"],
      scope: "user",
      origin: "Claude Code user settings · Cutokyo-owned entry",
      state: "enabled",
      managedByCutokyo: true,
      description: "Atomic spool capture; the hook never opens SQLite.",
      provenance: provenance("claude_code", "local_state"),
    },
    {
      id: "skill-protocol-check",
      kind: "skill",
      name: "protocol-check",
      harnesses: ["opencode"],
      scope: "project",
      origin: "Project skills directory",
      state: "enabled",
      managedByCutokyo: false,
      description: "User-owned skill discovered from harness configuration.",
      provenance: provenance("opencode", "local_state"),
    },
  ],
};

const baseGuards: GuardsResponse = {
  meta: {
    freshness: "partial",
    notices: [
      "Provider-bound request inspection is unavailable while proxy capture is disabled.",
    ],
    generatedAt: FIXTURE_NOW,
  },
  outgoingGuardEnabled: false,
  proxyEnabled: false,
  proxyStatus: "inactive",
  contextBreakdownAvailable: false,
  channels: [
    {
      id: "persisted",
      name: "On-disk persistence",
      category: "on_disk",
      state: "inspected",
      findings: 3,
      description:
        "Synthetic secrets are redacted before spool and SQLite persistence.",
      limitation:
        "Local transcripts remain plaintext after redaction; use full-disk encryption.",
    },
    {
      id: "telemetry",
      name: "Cutokyo telemetry",
      category: "telemetry",
      state: "disabled",
      findings: null,
      description:
        "Cutokyo telemetry is off. Disabled does not mean zero findings.",
      limitation: null,
    },
    {
      id: "bundle",
      name: "Diagnostic bundle",
      category: "bundle",
      state: "inspected",
      findings: 1,
      description:
        "Bundle projection excludes prompts, transcripts, raw secrets, and full project paths.",
      limitation: null,
    },
    {
      id: "provider",
      name: "Provider-bound requests",
      category: "provider_bound",
      state: "unavailable",
      findings: null,
      description:
        "Native capture does not inspect or mutate traffic sent to a model provider.",
      limitation:
        "Enable the outgoing guard and a supported inspection channel to block detected secrets.",
    },
  ],
};

const healthyDimensions = [
  "health_persistence",
  "quarantine",
  "spool_cap",
  "spool_drain",
  "writer_lock",
  "schema",
  "derive",
  "integrity",
  "rebuild",
  "backup",
  "restore",
].map((id) => ({
  id,
  name: id.replaceAll("_", " "),
  state: "healthy" as const,
  detail:
    id === "integrity"
      ? "Last integrity check: ok"
      : "Current operation is healthy.",
  actionLabel: null,
  lastSuccessAt: FIXTURE_NOW,
  lastFailureAt: null,
  failureCategory: null,
}));

const baseHealth: HealthResponse = {
  meta: { freshness: "complete", notices: [], generatedAt: FIXTURE_NOW },
  dimensions: healthyDimensions,
  currentQuarantineCount: 0,
  lifetimeQuarantineCount: 2,
  firstAffectedObservationId: null,
  drainPendingCount: 0,
  drainPendingBytes: 0,
  drainLagSeconds: 0,
  spoolCapReason: null,
  writerOwner: "desktop · pid 7319",
  schemaVersion: 2,
  deriveVersion: 1,
  lastIntegrityResult: "ok",
};

const degradedHealth: HealthResponse = {
  ...baseHealth,
  meta: {
    freshness: "degraded",
    notices: ["Writer lock and quarantine require separate recovery actions."],
    generatedAt: FIXTURE_NOW,
  },
  dimensions: healthyDimensions.map((dimension) =>
    dimension.id === "writer_lock"
      ? {
          ...dimension,
          state: "degraded" as const,
          detail:
            "The prior CLI owner released the lock, but desktop has not retried acquisition.",
          actionLabel: "Retry writer lock",
          lastFailureAt: "2026-09-19T16:40:00Z",
          failureCategory: "lock_contended",
        }
      : dimension.id === "quarantine"
        ? {
            ...dimension,
            state: "degraded" as const,
            detail:
              "One malformed spool entry is durably quarantined and capture continues.",
            actionLabel: "Open quarantine guidance",
            lastFailureAt: "2026-09-19T16:39:00Z",
            failureCategory: "invalid_contract",
          }
        : dimension,
  ),
  currentQuarantineCount: 1,
  lifetimeQuarantineCount: 7,
  firstAffectedObservationId: "spool:malformed-73A9",
  drainPendingCount: 3,
  drainPendingBytes: 8_192,
  drainLagSeconds: 94,
  writerOwner: "cli · pid 7301 (released)",
};

const baseSettings: DesktopSettings = {
  proxy_enabled: false,
  outgoing_guard_enabled: false,
  search_mcp_enabled: true,
  retention_days: null,
  updater_choice: "notify",
  crash_reports_enabled: false,
};

export interface FixtureState {
  onboarding: OnboardingResponse;
  dashboard: DashboardResponse;
  sessions: SessionRecord[];
  inventory: InventoryResponse;
  guards: GuardsResponse;
  health: HealthResponse;
  settings: DesktopSettings;
  analysisResults: AnalysisResult[];
  outboundAnalysisRequests: number;
  resumeRequests: string[];
  bundleCreated: boolean;
  failNextAnalysis: boolean;
}

export function createScenario(caseName: string | null): FixtureState {
  let onboarding = structuredClone(baseOnboarding);
  let dashboard = structuredClone(baseDashboard);
  let sessions = structuredClone([...dashboardSessions, analysisSession]);
  let health = structuredClone(baseHealth);

  if (caseName === "onboarding-empty" || caseName === "empty-history") {
    sessions = [];
    dashboard = {
      ...dashboard,
      meta: {
        freshness: "complete",
        notices:
          caseName === "onboarding-empty"
            ? ["Setup is incomplete; no operator history has been read."]
            : [],
        generatedAt: FIXTURE_NOW,
      },
      sessions: [],
      conflicts: [],
      captureLive: caseName === "empty-history",
    };
  }
  if (caseName === "onboarding-empty") {
    onboarding = {
      ...onboarding,
      complete: false,
      meta: { freshness: "complete", notices: [], generatedAt: FIXTURE_NOW },
    };
  }
  if (caseName === "retention-delete") {
    sessions = structuredClone([deleteSession, keepSession]);
  }
  if (caseName === "health-degraded") {
    health = structuredClone(degradedHealth);
  }

  return {
    onboarding,
    dashboard,
    sessions,
    inventory: structuredClone(baseInventory),
    guards: structuredClone(baseGuards),
    health,
    settings: structuredClone(baseSettings),
    analysisResults: [],
    outboundAnalysisRequests: 0,
    resumeRequests: [],
    bundleCreated: false,
    failNextAnalysis: caseName === "analysis-error",
  };
}

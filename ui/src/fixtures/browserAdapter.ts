import type { SettingsPatch } from "../generated/settings.js";
import type {
  ActionReceipt,
  AnalysisCandidate,
  AnalysisPreview,
  AnalysisResult,
  BootstrapResponse,
  BundlePreview,
  CommandClient,
  CompleteOnboardingRequest,
  DeletionPreview,
  DeletionReceipt,
  DesktopSettings,
  DoctorReport,
  HealthResponse,
  InventoryResponse,
  PluginVerification,
  ProxyPreview,
  ResumePreview,
  RetentionPreview,
  SessionFilters,
  SessionRecord,
  SessionSearchResponse,
  UpdateStatus,
} from "../contracts.js";
import { createScenario, FIXTURE_NOW, type FixtureState } from "./scenarios.js";

const JEV_CASE_SCENARIOS: Readonly<Record<string, string>> = {
  "onboarding-empty-history": "onboarding-empty",
  "search-detail-resume": "search-resume",
  "retention-delete-confirmation": "retention-delete",
  "guard-proxy-coverage-language": "guards-proxy",
  "analysis-preview-cancel": "analysis-cancel",
  "degraded-health-recovery": "health-degraded",
  "mcp-plugin-inventory": "mcp-plugin-inventory",
  "visual-keyboard-consistency": "visual-keyboard",
};

const VALID_CASES = new Set([
  ...Object.keys(JEV_CASE_SCENARIOS),
  ...Object.values(JEV_CASE_SCENARIOS),
  "analysis-error",
  "populated-dashboard",
  "empty-history",
]);

const DELETION_DISCLOSURE =
  "Deletion removes selected rows and search projections transactionally. It is not physical secure erasure from SQLite WAL history, SSD flash translation layers, filesystem snapshots, or existing backups. Existing backups are unchanged.";

function wait(milliseconds = 18): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function stableToken(prefix: string, values: readonly string[]): string {
  return `${prefix}:${[...values].sort().join("|") || "none"}:73A9`;
}

function deletionPreview(
  sessions: readonly SessionRecord[],
  prefix: string,
): DeletionPreview {
  const ids = sessions.map((session) => session.id);
  return {
    previewToken: stableToken(prefix, ids),
    sessionIds: ids,
    sessionTitles: sessions.map((session) => session.title ?? session.id),
    rawObservations: sessions.length * 3,
    messages: sessions.reduce(
      (total, session) => total + session.timeline.length,
      0,
    ),
    summaries: sessions.filter((session) => session.summary !== null).length,
    ftsRows: sessions.reduce(
      (total, session) => total + session.timeline.length,
      0,
    ),
    disclosure: DELETION_DISCLOSURE,
  };
}

function receipt(preview: DeletionPreview): DeletionReceipt {
  return {
    sessions: preview.sessionIds.length,
    rawObservations: preview.rawObservations,
    messages: preview.messages,
    summaries: preview.summaries,
    ftsRows: preview.ftsRows,
    disclosure: preview.disclosure,
  };
}

function includesText(session: SessionRecord, needle: string): boolean {
  const haystack = [
    session.id,
    session.nativeSessionKey,
    session.nativeResumeId,
    session.title,
    session.summary,
    session.project,
    session.branch,
    ...session.tools,
    ...session.skills,
    ...session.agents,
    ...session.timeline.flatMap((entry) => [entry.title, entry.body]),
  ]
    .filter((value): value is string => value !== null)
    .join(" ")
    .toLocaleLowerCase();
  return haystack.includes(needle.toLocaleLowerCase());
}

function search(state: FixtureState, filters: SessionFilters): SessionRecord[] {
  const fixedNow = new Date(FIXTURE_NOW).getTime();
  const rangeDays =
    filters.dateRange === "today"
      ? 1
      : filters.dateRange === "7d"
        ? 7
        : filters.dateRange === "30d"
          ? 30
          : filters.dateRange === "90d"
            ? 90
            : null;
  return state.sessions
    .filter(
      (session) => filters.text === "" || includesText(session, filters.text),
    )
    .filter(
      (session) =>
        filters.harness === "all" || session.harness === filters.harness,
    )
    .filter(
      (session) =>
        filters.project === "" || session.project === filters.project,
    )
    .filter(
      (session) => filters.branch === "" || session.branch === filters.branch,
    )
    .filter(
      (session) => filters.tool === "" || session.tools.includes(filters.tool),
    )
    .filter(
      (session) =>
        filters.skill === "" || session.skills.includes(filters.skill),
    )
    .filter(
      (session) =>
        filters.agent === "" || session.agents.includes(filters.agent),
    )
    .filter(
      (session) =>
        rangeDays === null ||
        new Date(session.startedAt).getTime() >=
          fixedNow - rangeDays * 86_400_000,
    )
    .sort((left, right) => right.startedAt.localeCompare(left.startedAt));
}

interface FixtureAudit {
  readonly outboundAnalysisRequests: number;
  readonly resumeRequests: readonly string[];
  readonly bundleCreated: boolean;
  readonly sessionIds: readonly string[];
}

export interface BrowserFixtureClient extends CommandClient {
  fixtureAudit(): FixtureAudit;
}

/**
 * Browser-only deterministic command adapter. It is imported behind a compile-time
 * development flag and never reads operator storage, Tauri APIs, or filesystem paths.
 */
export function createBrowserFixtureClient(
  caseName: string | null,
): BrowserFixtureClient {
  if (caseName !== null && !VALID_CASES.has(caseName)) {
    throw new Error(`Unknown isolated browser fixture: ${caseName}`);
  }

  const scenarioName =
    caseName === null ? null : (JEV_CASE_SCENARIOS[caseName] ?? caseName);
  const state = createScenario(scenarioName);
  const previews = new Map<string, DeletionPreview>();
  const analysisPreviews = new Map<string, AnalysisPreview>();
  const cancelledRequests = new Set<string>();

  async function getInventory(): Promise<InventoryResponse> {
    await wait();
    return structuredClone(state.inventory);
  }

  async function getHealth(): Promise<HealthResponse> {
    await wait();
    return structuredClone(state.health);
  }

  const client: BrowserFixtureClient = {
    async getBootstrap(): Promise<BootstrapResponse> {
      await wait();
      return {
        appVersion: "0.1.0-test",
        onboardingComplete: state.onboarding.complete,
        localOnly: !state.guards.proxyEnabled,
        proxyActive: state.guards.proxyStatus === "active",
        analysisEgressEnabled: false,
        writerMode: state.health.dimensions.some(
          (dimension) =>
            dimension.id === "writer_lock" && dimension.state === "degraded",
        )
          ? "read_only"
          : "owner",
        startupNotice: null,
        routeHint: state.onboarding.complete ? "/dashboard" : "/onboarding",
      };
    },
    async getOnboarding() {
      await wait();
      return structuredClone(state.onboarding);
    },
    async completeOnboarding(request: CompleteOnboardingRequest) {
      await wait(35);
      if (
        !request.acknowledgedPlaintextStorage ||
        request.harnesses.length === 0
      ) {
        throw new Error(
          "Acknowledge local plaintext storage and select a harness.",
        );
      }
      if (request.proxyEnabled || request.analysisEgressEnabled) {
        throw new Error("Onboarding cannot silently enable optional egress.");
      }
      state.onboarding = { ...state.onboarding, complete: true };
      return {
        ok: true,
        status: "success",
        message:
          "Local-only setup is ready. Proxy capture and AI analysis remain off.",
      };
    },
    async getDashboard() {
      await wait();
      return structuredClone({
        ...state.dashboard,
        sessions: state.dashboard.sessions,
      });
    },
    async searchSessions(
      filters: SessionFilters,
    ): Promise<SessionSearchResponse> {
      await wait();
      const sessions = search(state, filters);
      const distinct = (values: readonly (string | null)[]) =>
        [
          ...new Set(values.filter((value): value is string => value !== null)),
        ].sort();
      return {
        meta: {
          freshness: state.onboarding.complete ? "complete" : "partial",
          notices: state.onboarding.complete
            ? []
            : [
                "Setup is incomplete, so no native history has been captured yet.",
              ],
          generatedAt: FIXTURE_NOW,
        },
        sessions: structuredClone(sessions),
        total: sessions.length,
        availableProjects: distinct(
          state.sessions.map((session) => session.project),
        ),
        availableBranches: distinct(
          state.sessions.map((session) => session.branch),
        ),
        availableTools: distinct(
          state.sessions.flatMap((session) => session.tools),
        ),
        availableSkills: distinct(
          state.sessions.flatMap((session) => session.skills),
        ),
        availableAgents: distinct(
          state.sessions.flatMap((session) => session.agents),
        ),
      };
    },
    async getSession(sessionId: string) {
      await wait();
      const session = state.sessions.find(
        (candidate) => candidate.id === sessionId,
      );
      if (session === undefined)
        throw new Error(`Session ${sessionId} is unavailable.`);
      return structuredClone(session);
    },
    async previewResume(sessionId: string): Promise<ResumePreview> {
      const session = await client.getSession(sessionId);
      const harnessName =
        session.harness === "claude_code"
          ? "Claude Code"
          : session.harness === "codex"
            ? "Codex"
            : "OpenCode";
      return {
        sessionId,
        harness: session.harness,
        harnessName,
        nativeResumeId: session.nativeResumeId ?? "",
        commandDescription: `Open ${harnessName} with recorded native target ${session.nativeResumeId ?? "unavailable"}`,
        canResume: session.nativeResumeId !== null,
        unavailableReason:
          session.nativeResumeId === null
            ? "The source did not establish an exact native resume target."
            : null,
      };
    },
    async resumeSession(sessionId: string) {
      const preview = await client.previewResume(sessionId);
      if (!preview.canResume) {
        return {
          ok: false,
          status: "unavailable",
          message:
            preview.unavailableReason ?? "Exact native resume is unavailable.",
        };
      }
      state.resumeRequests.push(preview.nativeResumeId);
      return {
        ok: true,
        status: "success",
        message: `${preview.harnessName} received exact native target ${preview.nativeResumeId}.`,
      };
    },
    async previewSessionDeletion(sessionId: string) {
      const session = await client.getSession(sessionId);
      const preview = deletionPreview([session], "delete-session");
      previews.set(preview.previewToken, preview);
      return structuredClone(preview);
    },
    async deleteSession(sessionId: string, previewToken: string) {
      await wait();
      const preview = previews.get(previewToken);
      if (
        preview === undefined ||
        preview.sessionIds.length !== 1 ||
        preview.sessionIds[0] !== sessionId
      ) {
        throw new Error(
          "Deletion preview is missing, stale, or does not match this session.",
        );
      }
      state.sessions = state.sessions.filter(
        (session) => session.id !== sessionId,
      );
      previews.delete(previewToken);
      return receipt(preview);
    },
    async previewRetention(days: number): Promise<RetentionPreview> {
      await wait();
      if (!Number.isInteger(days) || days < 1 || days > 36_500) {
        throw new Error("Retention must be between 1 and 36500 days.");
      }
      const cutoffTime = new Date(FIXTURE_NOW).getTime() - days * 86_400_000;
      const selected = state.sessions.filter(
        (session) => new Date(session.startedAt).getTime() < cutoffTime,
      );
      const preview = {
        ...deletionPreview(selected, `retention-${days}`),
        retentionDays: days,
        cutoff: new Date(cutoffTime).toISOString(),
      };
      previews.set(preview.previewToken, preview);
      return structuredClone(preview);
    },
    async applyRetention(previewToken: string) {
      await wait();
      const preview = previews.get(previewToken);
      if (preview === undefined)
        throw new Error("Retention preview is missing or stale.");
      const selected = new Set(preview.sessionIds);
      state.sessions = state.sessions.filter(
        (session) => !selected.has(session.id),
      );
      previews.delete(previewToken);
      return receipt(preview);
    },
    async deleteAll(confirmation: string) {
      await wait();
      if (confirmation !== "DELETE ALL LOCAL HISTORY") {
        throw new Error(
          "Type DELETE ALL LOCAL HISTORY exactly before deleting all history.",
        );
      }
      const preview = deletionPreview(state.sessions, "delete-all");
      state.sessions = [];
      return receipt(preview);
    },
    getInventory,
    async setMcpEnabled(itemId: string, enabled: boolean) {
      await wait();
      const item = state.inventory.items.find(
        (candidate) => candidate.id === itemId,
      );
      if (item === undefined || item.kind !== "mcp") {
        throw new Error(`MCP server ${itemId} is unavailable.`);
      }
      state.inventory = {
        ...state.inventory,
        items: state.inventory.items.map((candidate) =>
          candidate.id === itemId
            ? {
                ...candidate,
                state: enabled ? ("enabled" as const) : ("disabled" as const),
              }
            : candidate,
        ),
      };
      return structuredClone(state.inventory);
    },
    async getPluginVerification(itemId: string): Promise<PluginVerification> {
      await wait();
      const item = state.inventory.items.find(
        (candidate) => candidate.id === itemId,
      );
      if (item === undefined || item.kind !== "plugin") {
        throw new Error(`Plugin ${itemId} is unavailable.`);
      }
      return {
        itemId,
        protocolMajor: 1,
        protocolState: item.state === "degraded" ? "unknown" : "compatible",
        capabilities: ["normalized_records:read", "tags:write"],
        transcriptApproved: false,
        networkApproved: false,
        limits: [
          { label: "Message line", value: "1 MiB" },
          { label: "Output records", value: "1,000 / request" },
          { label: "Execution timeout", value: "30 seconds" },
          { label: "Telemetry fields", value: "64 sanitized fields" },
        ],
        evidence: [
          "Valid protocol-major handshake",
          "Malformed-output recovery fixture passed",
          "Idempotent retry fixture passed",
        ],
        sandboxDisclosure:
          "Capabilities control data sent by Cutokyo. This verifier does not claim portable filesystem or network sandboxing where the operating system does not enforce it.",
      };
    },
    async getGuards() {
      await wait();
      return structuredClone(state.guards);
    },
    async previewProxy(): Promise<ProxyPreview> {
      await wait();
      return {
        consentToken: "proxy-consent:73A9",
        bindAddress: "127.0.0.1 (random local port)",
        inspectedContent: [
          "Provider request headers and URLs",
          "Prompt and tool payload chunks needed for context attribution",
          "Rate-limit response headers",
        ],
        neverPersisted: [
          "Authorization credentials",
          "Raw provider request bodies",
        ],
        fallbackBehavior:
          "Proxy capture is used only for facts native sources cannot establish. Instrumentation failure leaves the harness running.",
        guardBehavior:
          "If the outgoing guard is separately enabled, an uninspectable provider-bound channel is blocked rather than called protected.",
      };
    },
    async setProxyEnabled(enabled: boolean, consentToken: string | null) {
      await wait();
      if (enabled && consentToken !== "proxy-consent:73A9") {
        throw new Error(
          "Proxy capture requires the current explicit consent preview.",
        );
      }
      state.guards = {
        ...state.guards,
        proxyEnabled: enabled,
        proxyStatus: enabled ? "active" : "inactive",
        contextBreakdownAvailable: enabled,
        channels: state.guards.channels.map((channel) =>
          channel.id === "provider"
            ? {
                ...channel,
                state: enabled
                  ? ("inspected" as const)
                  : ("unavailable" as const),
                findings: enabled ? 0 : null,
              }
            : channel,
        ),
      };
      state.settings = { ...state.settings, proxy_enabled: enabled };
      return structuredClone(state.guards);
    },
    async setOutgoingGuardEnabled(enabled: boolean) {
      await wait();
      state.guards = { ...state.guards, outgoingGuardEnabled: enabled };
      state.settings = { ...state.settings, outgoing_guard_enabled: enabled };
      return structuredClone(state.guards);
    },
    async getAnalysisCandidates(): Promise<readonly AnalysisCandidate[]> {
      await wait();
      return state.sessions.map((session) => ({
        sessionId: session.id,
        title: session.title ?? session.id,
        harness: session.harness,
        coverage: session.provenance.coverage,
      }));
    },
    async previewAnalysis(
      sessionIds: readonly string[],
    ): Promise<AnalysisPreview> {
      await wait();
      if (sessionIds.length === 0)
        throw new Error("Select at least one session to analyze.");
      const selected = sessionIds.map((id) => {
        const session = state.sessions.find((candidate) => candidate.id === id);
        if (session === undefined)
          throw new Error(`Session ${id} is unavailable.`);
        return session;
      });
      const preview: AnalysisPreview = {
        previewToken: stableToken("analysis-preview", sessionIds),
        requestId: stableToken("analysis-request", sessionIds),
        sourceSessionIds: [...sessionIds],
        sourceSessionTitles: selected.map(
          (session) => session.title ?? session.id,
        ),
        provider: "Synthetic local QA provider",
        model: "fixture-summary-v1",
        promptVersion: "summary-prompt-1",
        payloadScope: [
          "Selected session titles and redacted message text",
          "Attributable tool names and completion states",
          "Coverage and provenance labels",
        ],
        redactions: [
          "Credential-like values",
          "Full local project paths",
          "Raw tool secrets",
        ],
        estimatedInputTokens: 1_240,
        estimatedPriceMicros: null,
        priceLabel:
          "No authoritative price is available for this synthetic provider.",
      };
      analysisPreviews.set(preview.previewToken, preview);
      return structuredClone(preview);
    },
    async runAnalysis(previewToken: string): Promise<AnalysisResult> {
      const preview = analysisPreviews.get(previewToken);
      if (preview === undefined)
        throw new Error("Analysis preview is missing or stale.");
      const existing = state.analysisResults.find(
        (result) =>
          result.idempotencyKey === `analysis:${preview.previewToken}`,
      );
      if (existing !== undefined) return structuredClone(existing);
      if (cancelledRequests.has(preview.requestId)) {
        throw new Error("Analysis was cancelled before any outbound request.");
      }
      state.outboundAnalysisRequests += 1;
      await wait(420);
      if (cancelledRequests.has(preview.requestId)) {
        throw new Error("Analysis was cancelled before a summary was written.");
      }
      if (state.failNextAnalysis) {
        state.failNextAnalysis = false;
        throw new Error(
          "Synthetic provider timed out. No summary was written; retry is safe.",
        );
      }
      const result: AnalysisResult = {
        summaryId: `summary:${preview.sourceSessionIds.join("+")}`,
        text: "The selected session reconciled native evidence, preserved partial coverage, and left optional egress disabled outside this confirmed request.",
        provider: preview.provider,
        model: preview.model,
        promptVersion: preview.promptVersion,
        sourceSessionIds: [...preview.sourceSessionIds],
        idempotencyKey: `analysis:${preview.previewToken}`,
        createdAt: FIXTURE_NOW,
      };
      state.analysisResults.push(result);
      return structuredClone(result);
    },
    async cancelAnalysis(requestId: string): Promise<ActionReceipt> {
      await wait();
      cancelledRequests.add(requestId);
      return {
        ok: true,
        status: "cancelled",
        message: "Analysis request cancelled.",
      };
    },
    getHealth,
    async retryHealth(dimensionId: string) {
      await wait(60);
      const dimensions = state.health.dimensions.map((dimension) =>
        dimension.id === dimensionId
          ? {
              ...dimension,
              state: "healthy" as const,
              detail: "Safe retry succeeded.",
              actionLabel: null,
              lastSuccessAt: FIXTURE_NOW,
            }
          : dimension,
      );
      const degradedDimensions = dimensions.filter(
        (dimension) => dimension.state === "degraded",
      );
      state.health = {
        ...state.health,
        dimensions,
        meta: {
          ...state.health.meta,
          notices: degradedDimensions.map(
            (dimension) => `${dimension.name} remains degraded.`,
          ),
          freshness: degradedDimensions.length === 0 ? "complete" : "degraded",
        },
        writerOwner:
          dimensionId === "writer_lock"
            ? "desktop · pid 7319"
            : state.health.writerOwner,
      };
      return structuredClone(state.health);
    },
    async runDoctor(): Promise<DoctorReport> {
      await wait(75);
      return {
        overall: state.health.dimensions.some(
          (dimension) => dimension.state === "degraded",
        )
          ? "degraded"
          : "healthy",
        checks: state.health.dimensions.map((dimension) => ({
          name: dimension.name,
          state: dimension.state,
          detail: dimension.detail,
        })),
      };
    },
    async previewBundle(): Promise<BundlePreview> {
      await wait();
      return {
        files: [
          "doctor.json",
          "health.json",
          "bounded-logs.jsonl",
          "version.txt",
        ],
        exclusions: [
          "Prompts",
          "Transcripts",
          "Raw secrets",
          "Full project paths",
          "API keys",
        ],
        redactions: [
          "Home directories → <home>",
          "Project paths → stable digest",
          "Tokens → [REDACTED]",
        ],
        estimatedBytes: 48_200,
      };
    },
    async createBundle() {
      await wait(70);
      state.bundleCreated = true;
      return {
        ok: true,
        status: "success",
        message:
          "Diagnostic bundle created in the application-managed exports folder.",
      };
    },
    async getSettings(): Promise<DesktopSettings> {
      await wait();
      return structuredClone(state.settings);
    },
    async patchSettings(patch: SettingsPatch) {
      await wait();
      state.settings = {
        ...state.settings,
        ...(patch.proxy_enabled === undefined
          ? {}
          : { proxy_enabled: patch.proxy_enabled }),
        ...(patch.outgoing_guard_enabled === undefined
          ? {}
          : { outgoing_guard_enabled: patch.outgoing_guard_enabled }),
        ...(patch.search_mcp_enabled === undefined
          ? {}
          : { search_mcp_enabled: patch.search_mcp_enabled }),
        ...(patch.retention_days === undefined
          ? {}
          : { retention_days: patch.retention_days }),
      };
      state.guards = {
        ...state.guards,
        outgoingGuardEnabled: state.settings.outgoing_guard_enabled,
      };
      state.inventory = {
        ...state.inventory,
        searchMcpEnabled: state.settings.search_mcp_enabled,
      };
      return structuredClone(state.settings);
    },
    async patchDesktopPreferences(patch) {
      await wait();
      state.settings = { ...state.settings, ...patch };
      return structuredClone(state.settings);
    },
    async checkForUpdates(): Promise<UpdateStatus> {
      await wait(90);
      return {
        state: "current",
        currentVersion: "0.1.0-test",
        availableVersion: null,
        checkedAt: FIXTURE_NOW,
        detail: "No signed update is available on the selected channel.",
      };
    },
    fixtureAudit() {
      return {
        outboundAnalysisRequests: state.outboundAnalysisRequests,
        resumeRequests: [...state.resumeRequests],
        bundleCreated: state.bundleCreated,
        sessionIds: state.sessions.map((session) => session.id),
      };
    },
  };

  return client;
}

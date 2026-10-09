import type { SettingsPatch } from "../generated/settings.js";
import type {
  BootstrapResponse,
  BundlePreview,
  CommandClient,
  CompleteOnboardingRequest,
  CaptureSetupPreview,
  Harness,
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
import { localMidnight, sessionMatches } from "../domain/sessionSearch.js";
import { createScenario, FIXTURE_NOW, type FixtureState } from "./scenarios.js";
import { createInventoryFixtureManagement } from "./inventoryManagement.js";
import { createBackupFixtureManagement } from "./backupManagement.js";

const JEV_CASE_SCENARIOS: Readonly<Record<string, string>> = {
  "onboarding-empty-history": "onboarding-empty",
  "search-detail-resume": "search-resume",
  "retention-delete-confirmation": "retention-delete",
  "proxy-capture-consent": "proxy-capture",
  "degraded-health-recovery": "health-degraded",
  "mcp-plugin-inventory": "mcp-plugin-inventory",
  "visual-keyboard-consistency": "visual-keyboard",
};

const VALID_CASES = new Set([
  ...Object.keys(JEV_CASE_SCENARIOS),
  ...Object.values(JEV_CASE_SCENARIOS),
  "search-pagination",
  "populated-dashboard",
  "empty-history",
  "appearance-dark",
  "appearance-dark-delayed",
  "appearance-conflict",
  "appearance-save-error",
  "native-capabilities-unavailable",
  "large-inventory",
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
    // Declared fixture evidence has three raw observations per session; search
    // also stores one canonical-message document per message and one per session.
    ftsRows:
      sessions.length * 4 +
      sessions.reduce((total, session) => total + session.timeline.length, 0),
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

function search(state: FixtureState, filters: SessionFilters): SessionRecord[] {
  const fixedNow = new Date(FIXTURE_NOW).getTime();
  const rangeDays =
    filters.dateRange === "7d"
      ? 7
      : filters.dateRange === "30d"
        ? 30
        : filters.dateRange === "90d"
          ? 90
          : null;
  const since =
    filters.dateRange === "today"
      ? new Date(localMidnight(new Date(FIXTURE_NOW))).getTime()
      : rangeDays === null
        ? null
        : fixedNow - rangeDays * 86_400_000;
  const scored = new Map<string, number>();
  return state.sessions
    .map((session) => {
      const result = sessionMatches(session, filters);
      scored.set(session.id, result.score);
      return { ...session, matches: result.matches, matched: result.matched };
    })
    .filter((session) => session.matched)
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
        since === null || new Date(session.startedAt).getTime() >= since,
    )
    .sort(
      (left, right) =>
        (filters.sort === "relevance" && filters.text.trim() !== ""
          ? (scored.get(right.id) ?? 0) - (scored.get(left.id) ?? 0)
          : 0) ||
        right.startedAt.localeCompare(left.startedAt) ||
        left.id.localeCompare(right.id),
    )
    .map(({ matched: _matched, ...session }) => session);
}

interface FixtureAudit {
  readonly proxyActive: boolean;
  readonly resumeRequests: readonly string[];
  readonly bundleCreated: boolean;
  readonly sessionIds: readonly string[];
}

export interface BrowserFixtureClient extends CommandClient {
  fixtureAudit(): FixtureAudit;
  /** Simulates the desktop finishing an import that stored new history. */
  emitHistoryImported(): void;
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
  const capturePreviews = new Map<string, CaptureSetupPreview>();
  const installedCapture = new Set<Harness>();
  let captureSequence = 0;
  const previews = new Map<string, DeletionPreview>();
  const historyListeners = new Set<() => void>();
  let holdFirstAppearanceRead = caseName === "appearance-dark-delayed";

  async function getInventory(): Promise<InventoryResponse> {
    await wait();
    return structuredClone(state.inventory);
  }

  async function getHealth(): Promise<HealthResponse> {
    await wait();
    return structuredClone(state.health);
  }

  const client: BrowserFixtureClient = {
    async getCapabilities() {
      await wait();
      const unavailable = caseName === "native-capabilities-unavailable";
      return {
        updates: {
          available: !unavailable,
          reason: unavailable
            ? "Signed updater metadata is not configured in this build. Install updates manually."
            : null,
        },
      };
    },
    async getBootstrap(): Promise<BootstrapResponse> {
      await wait();
      return {
        appVersion: "0.1.0-test",
        onboardingComplete: state.onboarding.complete,
        localOnly: !state.proxy.proxyEnabled,
        proxyActive: state.proxy.proxyStatus === "active",
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
    async previewCaptureSetup(harness, operation) {
      await wait();
      const preview: CaptureSetupPreview = {
        previewToken: `fixture-capture-${++captureSequence}`,
        harness,
        operation,
        version:
          harness === "claude_code"
            ? "2.1.278"
            : harness === "codex"
              ? "0.153.4"
              : "1.18.28",
        targets: [
          "Isolated browser fixture configuration. No native files are read or changed.",
        ],
        actions: [
          operation === "install"
            ? "Install owned native capture integration"
            : operation === "uninstall"
              ? "Remove only owned capture integration"
              : "No interrupted intent. No changes.",
        ],
        issues: [],
        verified: installedCapture.has(harness),
        recoveryPending: false,
        disclosure:
          "Browser simulation only. Native installation is not proven. Configuration verification does not establish live capture or transcript coverage.",
      };
      capturePreviews.set(preview.previewToken, preview);
      return structuredClone(preview);
    },
    async applyCaptureSetup(previewToken) {
      await wait();
      const preview = capturePreviews.get(previewToken);
      if (preview === undefined)
        throw new Error("Setup preview expired. Preview again.");
      capturePreviews.delete(previewToken);
      if (preview.operation === "install")
        installedCapture.add(preview.harness);
      if (preview.operation === "uninstall")
        installedCapture.delete(preview.harness);
      return {
        harness: preview.harness,
        operation: preview.operation,
        changed: preview.operation !== "recover",
        verified: installedCapture.has(preview.harness),
        issues: [],
        message:
          "Browser fixture updated only. No native configuration changed. Live capture remains unknown.",
      };
    },
    async completeOnboarding(request: CompleteOnboardingRequest) {
      await wait(35);
      if (!request.acknowledgedPlaintextStorage)
        throw new Error("Acknowledge local plaintext storage.");
      if (request.mode === "browse" && request.harnesses.length !== 0)
        throw new Error("Browse-only cannot select capture harnesses.");
      if (
        request.mode === "install" &&
        (request.harnesses.length === 0 ||
          request.harnesses.some((harness) => !installedCapture.has(harness)))
      )
        throw new Error(
          "Preview, install and verify every selected capture integration first.",
        );
      if (request.proxyEnabled || request.analysisEgressEnabled) {
        throw new Error("Onboarding cannot silently enable optional egress.");
      }
      state.onboarding = { ...state.onboarding, complete: true };
      return {
        ok: true,
        status: "success",
        message: "Local-only setup is ready. Proxy capture remains off.",
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
      const offset = Math.max(0, Math.trunc(filters.offset));
      const limit = Math.min(500, filters.limit || 50);
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
        sessions: structuredClone(sessions.slice(offset, offset + limit)),
        total: sessions.length,
        offset,
        limit,
        hasMore: offset + limit < sessions.length,
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
        projectDirectory: null,
        projectContextKnown: false,
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
        message: `Browser simulation recorded exact native target ${preview.nativeResumeId}. No terminal or harness was launched.`,
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
    ...createInventoryFixtureManagement(state),
    ...createBackupFixtureManagement(state, () => {
      previews.clear();
    }),
    onHistoryImported(listener) {
      historyListeners.add(listener);
      return () => historyListeners.delete(listener);
    },
    emitHistoryImported() {
      for (const listener of historyListeners) listener();
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
    async getProxyStatus() {
      await wait();
      return structuredClone(state.proxy);
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
        fallbackBehavior: state.proxy.detail,
        redactionBehavior:
          "Provider requests pass unchanged. Baseline redaction applies only to retained local trace metadata; credentials and payloads are not persisted.",
      };
    },
    async setProxyEnabled(enabled: boolean, consentToken: string | null) {
      await wait();
      if (enabled && consentToken !== "proxy-consent:73A9") {
        throw new Error(
          "Proxy capture requires the current explicit consent preview.",
        );
      }
      if (enabled) throw new Error(state.proxy.detail);
      state.proxy = {
        ...state.proxy,
        proxyEnabled: false,
        proxyStatus: "unavailable",
        contextBreakdownAvailable: false,
      };
      state.settings = { ...state.settings, proxy_enabled: false };
      return structuredClone(state.proxy);
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
        files: ["cutokyo-diagnostic-bundle.json"],
        exclusions: [
          "Prompts",
          "Transcripts",
          "Raw secrets",
          "Full project paths",
          "API keys",
        ],
        redactions: [
          "Only categorical health statuses and numeric counters are included",
          "The complete report receives baseline secret redaction",
        ],
        estimatedBytes: 16_384,
      };
    },
    async createBundle() {
      await wait(70);
      state.bundleCreated = true;
      return {
        ok: true,
        status: "success",
        message:
          "Created local diagnostic report at App data/diagnostics/fixture-export/cutokyo-diagnostic-bundle.json.",
      };
    },
    async getSettings(): Promise<DesktopSettings> {
      if (holdFirstAppearanceRead) {
        holdFirstAppearanceRead = false;
        await new Promise<void>((release) => {
          Object.defineProperty(
            globalThis,
            "__CUTOKYO_FIXTURE_RELEASE_APPEARANCE__",
            { configurable: true, value: () => release() },
          );
        });
      }
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
        ...(patch.search_mcp_enabled === undefined
          ? {}
          : { search_mcp_enabled: patch.search_mcp_enabled }),
        ...(patch.retention_days === undefined
          ? {}
          : { retention_days: patch.retention_days }),
      };
      state.proxy = {
        ...state.proxy,
        proxyEnabled: state.settings.proxy_enabled,
        proxyStatus: "unavailable",
      };
      state.inventory = {
        ...state.inventory,
        searchMcpEnabled: state.settings.search_mcp_enabled,
      };
      return structuredClone(state.settings);
    },
    async patchDesktopPreferences(patch) {
      await wait();
      if (patch.appearance !== undefined && state.failAppearanceSave) {
        throw new Error("Synthetic appearance preference could not be saved.");
      }
      if (
        patch.appearance !== undefined &&
        caseName === "appearance-conflict"
      ) {
        state.settings = { ...state.settings, appearance: "system" };
        throw new Error(
          "Desktop settings changed in another window; reload before saving.",
        );
      }
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
        proxyActive: state.proxy.proxyStatus === "active",
        resumeRequests: [...state.resumeRequests],
        bundleCreated: state.bundleCreated,
        sessionIds: state.sessions.map((session) => session.id),
      };
    },
  };

  return client;
}

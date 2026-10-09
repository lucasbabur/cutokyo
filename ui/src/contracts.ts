import type { SettingsPatch } from "./generated/settings.js";

export type Harness = "claude_code" | "codex" | "opencode";
type CoverageState =
  "complete" | "partial" | "disabled" | "unavailable" | "unknown_version";
type Confidence =
  "observed" | "estimated" | "user_declared" | "conflicting" | "unknown";
type RouteFreshness = "complete" | "partial" | "degraded";
type ItemState =
  "enabled" | "disabled" | "degraded" | "unknown" | "configured" | "installed";
export type InventoryKind = "mcp" | "skill" | "hook" | "plugin" | "instruction";
export type HealthState = "healthy" | "degraded" | "unknown";

export interface Coverage {
  readonly state: CoverageState;
  readonly scope: string;
  readonly gaps: readonly string[];
}

export interface Provenance {
  readonly channel: string;
  readonly sourceTier: number;
  readonly capturedAt: string;
  readonly parserVersion: string;
  readonly confidence: Confidence;
  readonly coverage: Coverage;
  /** May list only the first records; `observationCount` is the total. */
  readonly observationIds: readonly string[];
  readonly observationCount?: number;
}

export interface RouteMeta {
  readonly freshness: RouteFreshness;
  readonly notices: readonly string[];
  readonly generatedAt: string;
}

interface HarnessCoverage {
  readonly harness: Harness;
  readonly displayName: string;
  readonly capture: Coverage;
  readonly inventory: Coverage;
  readonly transcript: Coverage;
  readonly resume: Coverage;
  readonly source: string;
  readonly lastSeenAt: string | null;
  readonly actionable: string | null;
}

export interface BootstrapResponse {
  readonly appVersion: string;
  readonly onboardingComplete: boolean;
  readonly localOnly: boolean;
  readonly proxyActive: boolean;
  readonly analysisEgressEnabled: boolean;
  readonly writerMode: "owner" | "read_only" | "unavailable";
  readonly startupNotice: string | null;
  readonly routeHint: RoutePath;
}

export interface OnboardingResponse {
  readonly meta: RouteMeta;
  readonly complete: boolean;
  readonly storageDisclosure: string;
  readonly telemetryDisclosure: string;
  readonly harnesses: readonly HarnessCoverage[];
}

export type CaptureSetupOperation = "install" | "recover" | "uninstall";

export interface CaptureSetupPreview {
  readonly previewToken: string;
  readonly harness: Harness;
  readonly operation: CaptureSetupOperation;
  readonly version: string;
  readonly targets: readonly string[];
  readonly actions: readonly string[];
  readonly issues: readonly string[];
  readonly verified: boolean;
  readonly recoveryPending: boolean;
  readonly disclosure: string;
}

export interface CaptureSetupReceipt {
  readonly harness: Harness;
  readonly operation: CaptureSetupOperation;
  readonly changed: boolean;
  readonly verified: boolean;
  readonly issues: readonly string[];
  readonly message: string;
}

export interface CompleteOnboardingRequest {
  readonly mode: "install" | "browse";
  readonly harnesses: readonly Harness[];
  readonly acknowledgedPlaintextStorage: boolean;
  readonly proxyEnabled: false;
  readonly analysisEgressEnabled: false;
}

export interface UsageRecord {
  readonly nativeUsageKey: string;
  readonly harness: Harness;
  readonly model: string | null;
  readonly inputTokens: number | null;
  readonly outputTokens: number | null;
  readonly cacheReadTokens: number | null;
  readonly cacheWriteTokens: number | null;
  readonly providerCostMicros: number | null;
  readonly billingBasis:
    | "provider_reported"
    | "token_price"
    | "subscription"
    | "included"
    | "user_declared"
    | "unknown";
  readonly provenance: Provenance;
}

interface TimelineEntry {
  readonly id: string;
  readonly kind: "user" | "assistant" | "tool" | "agent" | "system" | "unknown";
  readonly at: string;
  readonly title: string;
  readonly body: string | null;
  readonly state: "succeeded" | "failed" | "running" | "cancelled" | "unknown";
  readonly provenance: Provenance;
}

export interface SessionRecord {
  readonly id: string;
  readonly harness: Harness;
  readonly nativeSessionKey: string;
  readonly nativeResumeId: string | null;
  readonly title: string | null;
  readonly summary: string | null;
  readonly matches: readonly SessionMatch[];
  readonly project: string | null;
  readonly branch: string | null;
  readonly startedAt: string;
  readonly endedAt: string | null;
  readonly state: "active" | "completed" | "interrupted" | "unknown";
  readonly tools: readonly string[];
  readonly skills: readonly string[];
  readonly agents: readonly string[];
  readonly usage: readonly UsageRecord[];
  readonly timeline: readonly TimelineEntry[];
  readonly provenance: Provenance;
}

export interface PriceFact {
  readonly provider: string;
  readonly model: string;
  readonly currency: string;
  readonly inputMicrosPerMillion: number | null;
  readonly outputMicrosPerMillion: number | null;
  readonly validFrom: string;
  readonly validUntil: string | null;
  readonly confidence: Confidence;
  readonly label: string;
  readonly provenance: Provenance;
}

export interface QuotaFact {
  readonly name: string;
  readonly limit: number | null;
  readonly used: number | null;
  readonly remaining: number | null;
  readonly resetsAt: string | null;
  readonly confidence: Confidence;
  readonly label: string;
  readonly provenance: Provenance;
}

interface SourceConflict {
  readonly fact: string;
  readonly winner: string;
  readonly ignored: string;
  readonly reason: string;
}

export interface DashboardResponse {
  readonly meta: RouteMeta;
  readonly sessions: readonly SessionRecord[];
  readonly prices: readonly PriceFact[];
  readonly quotas: readonly QuotaFact[];
  readonly contextBreakdown: {
    readonly instructionsTokens: number | null;
    readonly userTokens: number | null;
    readonly assistantTokens: number | null;
    readonly toolTokens: number | null;
    readonly cacheTokens: number | null;
    readonly remainingTokens: number | null;
    readonly provenance: Provenance;
  } | null;
  readonly conflicts: readonly SourceConflict[];
  readonly captureLive: boolean;
}

export interface SessionMatch {
  readonly source: "title" | "project" | "branch" | "native_id" | "transcript";
  /** Bounded plain text, never HTML. */
  readonly text: string;
  readonly terms: readonly string[];
}

export interface SessionFilters {
  readonly text: string;
  readonly queryMode: "terms" | "phrase";
  readonly sort: "relevance" | "newest";
  readonly offset: number;
  readonly limit: number;
  /** Local calendar midnight for Today, supplied by the command client. */
  readonly todayStart: string | null;
  readonly harness: Harness | "all";
  readonly project: string;
  readonly branch: string;
  readonly dateRange: "all" | "today" | "7d" | "30d" | "90d";
  readonly tool: string;
  readonly skill: string;
  readonly agent: string;
}

export interface SessionSearchResponse {
  readonly meta: RouteMeta;
  readonly sessions: readonly SessionRecord[];
  readonly total: number;
  readonly offset: number;
  readonly limit: number;
  readonly hasMore: boolean;
  readonly availableProjects: readonly string[];
  readonly availableBranches: readonly string[];
  readonly availableTools: readonly string[];
  readonly availableSkills: readonly string[];
  readonly availableAgents: readonly string[];
}

export interface ResumePreview {
  readonly sessionId: string;
  readonly harness: Harness;
  readonly harnessName: string;
  readonly nativeResumeId: string;
  readonly commandDescription: string;
  readonly projectDirectory: string | null;
  readonly projectContextKnown: boolean;
  readonly canResume: boolean;
  readonly unavailableReason: string | null;
}

export interface ActionReceipt {
  /** Whether the requested action completed; warning severity does not decide this. */
  readonly ok: boolean;
  readonly message: string;
  /** Acknowledged native resume warns that session activation remains unconfirmed. */
  readonly status: "success" | "warning" | "cancelled" | "unavailable";
}

export interface DeletionPreview {
  readonly previewToken: string;
  readonly sessionIds: readonly string[];
  readonly sessionTitles: readonly string[];
  readonly rawObservations: number;
  readonly messages: number;
  readonly summaries: number;
  readonly ftsRows: number;
  readonly disclosure: string;
}

export interface DeletionReceipt {
  readonly sessions: number;
  readonly rawObservations: number;
  readonly messages: number;
  readonly summaries: number;
  readonly ftsRows: number;
  readonly disclosure: string;
}

export interface RetentionPreview extends DeletionPreview {
  readonly retentionDays: number;
  readonly cutoff: string;
}

export interface InventoryItem {
  readonly id: string;
  readonly kind: InventoryKind;
  readonly name: string;
  readonly harnesses: readonly Harness[];
  readonly scope: "user" | "project" | "managed";
  readonly origin: string;
  readonly state: ItemState;
  readonly managedByCutokyo: boolean;
  readonly description: string;
  /** Canonical directory behind a symlinked skill; null for ordinary sources. */
  readonly linkTarget?: string | null;
  readonly provenance: Provenance;
}

export interface InventoryResponse {
  readonly meta: RouteMeta;
  readonly items: readonly InventoryItem[];
  readonly brokerState: HealthState;
  readonly searchMcpEnabled: boolean;
}

export interface InventoryDocument {
  readonly itemId: string;
  readonly content: string;
  readonly format: "markdown" | "json" | "toml" | "text";
  readonly revision: string;
  readonly editable: boolean;
  readonly removable: boolean;
  readonly unavailableReason: string | null;
  readonly installTargets: readonly {
    readonly harness: Harness;
    readonly available: boolean;
    readonly reason: string | null;
    readonly destination: string;
    /** Source fields with no equivalent here; names only, never values. */
    readonly droppedFields?: readonly string[];
    /** Field mappings and cautions applied by the conversion. */
    readonly conversions?: readonly string[];
  }[];
  /** Scope, sharing, link and bundle disclosures kept out of the row description. */
  readonly notes?: readonly string[];
}

export interface PluginVerification {
  readonly itemId: string;
  readonly protocolMajor: number;
  readonly protocolState: "compatible" | "rejected" | "unknown";
  readonly capabilities: readonly string[];
  readonly transcriptApproved: boolean;
  readonly networkApproved: boolean;
  readonly limits: readonly {
    readonly label: string;
    readonly value: string;
  }[];
  readonly evidence: readonly string[];
  readonly sandboxDisclosure: string;
}

export interface ProxyStatus {
  readonly meta: RouteMeta;
  readonly proxyEnabled: boolean;
  readonly proxyStatus: "inactive" | "active" | "unavailable";
  readonly detail: string;
  readonly contextBreakdownAvailable: boolean;
}

export interface ProxyPreview {
  readonly consentToken: string;
  readonly bindAddress: string;
  readonly inspectedContent: readonly string[];
  readonly neverPersisted: readonly string[];
  readonly fallbackBehavior: string;
  readonly redactionBehavior: string;
}

interface HealthDimension {
  readonly id: string;
  readonly name: string;
  readonly state: HealthState;
  readonly detail: string;
  readonly actionLabel: string | null;
  readonly lastSuccessAt: string | null;
  readonly lastFailureAt: string | null;
  readonly failureCategory: string | null;
}

export interface HealthResponse {
  readonly meta: RouteMeta;
  readonly dimensions: readonly HealthDimension[];
  readonly currentQuarantineCount: number;
  readonly lifetimeQuarantineCount: number;
  readonly firstAffectedObservationId: string | null;
  readonly drainPendingCount: number;
  readonly drainPendingBytes: number;
  readonly drainLagSeconds: number | null;
  readonly spoolCapReason: string | null;
  readonly writerOwner: string | null;
  readonly schemaVersion: number;
  readonly deriveVersion: number;
  readonly lastIntegrityResult: string | null;
}

export interface DoctorReport {
  readonly overall: HealthState;
  readonly checks: readonly {
    readonly name: string;
    readonly state: HealthState;
    readonly detail: string;
  }[];
}

export interface BundlePreview {
  readonly files: readonly string[];
  readonly exclusions: readonly string[];
  readonly redactions: readonly string[];
  readonly estimatedBytes: number;
}

export interface BackupInfo {
  readonly path: string;
  readonly createdAt: string;
  readonly byteLength: number;
  readonly sessionCount: number;
  readonly scope: string;
}

export interface BackupRestorePreview {
  readonly previewToken: string;
  readonly backup: BackupInfo;
  readonly currentSessionCount: number;
}

export interface BackupRestoreReceipt {
  readonly recoveryPath: string;
  readonly restoredSessionCount: number;
  readonly integrityResult: string;
}

export interface DesktopSettings {
  readonly proxy_enabled: boolean;
  readonly search_mcp_enabled: boolean;
  readonly retention_days: number | null;
  readonly updater_choice: "automatic" | "notify" | "manual";
  readonly crash_reports_enabled: boolean;
  readonly appearance: "system" | "light" | "dark";
}

export interface DesktopCapabilities {
  readonly updates: {
    readonly available: boolean;
    readonly reason: string | null;
  };
}

export interface UpdateStatus {
  readonly state: "current" | "available" | "unavailable" | "checking";
  readonly currentVersion: string;
  readonly availableVersion: string | null;
  readonly checkedAt: string | null;
  readonly detail: string;
}

export type RoutePath =
  | "/onboarding"
  | "/dashboard"
  | "/sessions"
  | "/inventory"
  | "/health"
  | "/data"
  | "/settings";

/** Applies the public omission-preserving patch semantics used by desktop forms. */
export function applySettingsPatch<T extends Readonly<Record<string, unknown>>>(
  current: T,
  patch: SettingsPatch,
): T & SettingsPatch {
  return { ...current, ...patch };
}

export interface CommandClient {
  getCapabilities(): Promise<DesktopCapabilities>;
  getBootstrap(): Promise<BootstrapResponse>;
  getOnboarding(): Promise<OnboardingResponse>;
  previewCaptureSetup(
    harness: Harness,
    operation: CaptureSetupOperation,
  ): Promise<CaptureSetupPreview>;
  applyCaptureSetup(previewToken: string): Promise<CaptureSetupReceipt>;
  completeOnboarding(
    request: CompleteOnboardingRequest,
  ): Promise<ActionReceipt>;
  getDashboard(): Promise<DashboardResponse>;
  searchSessions(filters: SessionFilters): Promise<SessionSearchResponse>;
  getSession(sessionId: string): Promise<SessionRecord>;
  previewResume(sessionId: string): Promise<ResumePreview>;
  resumeSession(sessionId: string): Promise<ActionReceipt>;
  previewSessionDeletion(sessionId: string): Promise<DeletionPreview>;
  deleteSession(
    sessionId: string,
    previewToken: string,
  ): Promise<DeletionReceipt>;
  previewRetention(days: number): Promise<RetentionPreview>;
  applyRetention(previewToken: string): Promise<DeletionReceipt>;
  deleteAll(confirmation: string): Promise<DeletionReceipt>;
  createBackup(destination?: string): Promise<BackupInfo>;
  listBackups(): Promise<readonly BackupInfo[]>;
  previewBackupRestore(path: string): Promise<BackupRestorePreview>;
  restoreBackup(previewToken: string): Promise<BackupRestoreReceipt>;
  getInventory(): Promise<InventoryResponse>;
  getInventoryDocument(itemId: string): Promise<InventoryDocument>;
  saveInventoryDocument(
    itemId: string,
    revision: string,
    content: string,
  ): Promise<ActionReceipt>;
  removeInventoryItem(itemId: string, revision: string): Promise<ActionReceipt>;
  installInventoryItem(
    itemId: string,
    revision: string,
    harness: Harness,
  ): Promise<ActionReceipt>;
  /** Turns an MCP server on or off; its configuration is kept either way. */
  setInventoryItemEnabled(
    itemId: string,
    revision: string,
    enabled: boolean,
  ): Promise<ActionReceipt>;
  getPluginVerification(itemId: string): Promise<PluginVerification>;
  /** Calls `listener` whenever a background import stores new history; returns an unsubscribe. */
  onHistoryImported(listener: () => void): () => void;
  getProxyStatus(): Promise<ProxyStatus>;
  previewProxy(): Promise<ProxyPreview>;
  setProxyEnabled(
    enabled: boolean,
    consentToken: string | null,
  ): Promise<ProxyStatus>;
  getHealth(): Promise<HealthResponse>;
  retryHealth(dimensionId: string): Promise<HealthResponse>;
  runDoctor(): Promise<DoctorReport>;
  previewBundle(): Promise<BundlePreview>;
  createBundle(): Promise<ActionReceipt>;
  getSettings(): Promise<DesktopSettings>;
  patchSettings(patch: SettingsPatch): Promise<DesktopSettings>;
  patchDesktopPreferences(
    patch: Readonly<
      Partial<
        Pick<
          DesktopSettings,
          "updater_choice" | "crash_reports_enabled" | "appearance"
        >
      >
    >,
  ): Promise<DesktopSettings>;
  checkForUpdates(): Promise<UpdateStatus>;
}

import { invoke } from "@tauri-apps/api/core";

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
  DashboardResponse,
  DeletionPreview,
  DeletionReceipt,
  DesktopSettings,
  DoctorReport,
  GuardsResponse,
  HealthResponse,
  InventoryResponse,
  OnboardingResponse,
  PluginVerification,
  ProxyPreview,
  ResumePreview,
  RetentionPreview,
  SessionFilters,
  SessionRecord,
  SessionSearchResponse,
  UpdateStatus,
} from "../contracts.js";

function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(name, args);
}

/** The production adapter exposes only named application use cases over Tauri IPC. */
export function createTauriCommandClient(): CommandClient {
  return {
    getBootstrap: () => command<BootstrapResponse>("desktop_bootstrap"),
    getOnboarding: () => command<OnboardingResponse>("onboarding_status"),
    completeOnboarding: (request: CompleteOnboardingRequest) =>
      command<ActionReceipt>("complete_onboarding", { request }),
    getDashboard: () => command<DashboardResponse>("dashboard_query"),
    searchSessions: (filters: SessionFilters) =>
      command<SessionSearchResponse>("search_sessions", { filters }),
    getSession: (sessionId: string) =>
      command<SessionRecord>("session_detail", { sessionId }),
    previewResume: (sessionId: string) =>
      command<ResumePreview>("preview_resume", { sessionId }),
    resumeSession: (sessionId: string) =>
      command<ActionReceipt>("resume_session", { sessionId }),
    previewSessionDeletion: (sessionId: string) =>
      command<DeletionPreview>("preview_session_deletion", { sessionId }),
    deleteSession: (sessionId: string, previewToken: string) =>
      command<DeletionReceipt>("delete_session", { sessionId, previewToken }),
    previewRetention: (days: number) =>
      command<RetentionPreview>("preview_retention", { days }),
    applyRetention: (previewToken: string) =>
      command<DeletionReceipt>("apply_retention", { previewToken }),
    deleteAll: (confirmation: string) =>
      command<DeletionReceipt>("delete_all_history", { confirmation }),
    getInventory: () => command<InventoryResponse>("inventory_query"),
    setMcpEnabled: (itemId: string, enabled: boolean) =>
      command<InventoryResponse>("set_mcp_enabled", { itemId, enabled }),
    getPluginVerification: (itemId: string) =>
      command<PluginVerification>("plugin_verification", { itemId }),
    getGuards: () => command<GuardsResponse>("guard_coverage"),
    previewProxy: () => command<ProxyPreview>("preview_proxy_consent"),
    setProxyEnabled: (enabled: boolean, consentToken: string | null) =>
      command<GuardsResponse>("set_proxy_enabled", { enabled, consentToken }),
    setOutgoingGuardEnabled: (enabled: boolean) =>
      command<GuardsResponse>("set_outgoing_guard_enabled", { enabled }),
    getAnalysisCandidates: () =>
      command<readonly AnalysisCandidate[]>("analysis_candidates"),
    previewAnalysis: (sessionIds: readonly string[]) =>
      command<AnalysisPreview>("preview_analysis", { sessionIds }),
    runAnalysis: (previewToken: string) =>
      command<AnalysisResult>("run_analysis", { previewToken }),
    cancelAnalysis: (requestId: string) =>
      command<ActionReceipt>("cancel_analysis", { requestId }),
    getHealth: () => command<HealthResponse>("health_snapshot"),
    retryHealth: (dimensionId: string) =>
      command<HealthResponse>("retry_health_dimension", { dimensionId }),
    runDoctor: () => command<DoctorReport>("run_doctor"),
    previewBundle: () => command<BundlePreview>("preview_diagnostic_bundle"),
    createBundle: () => command<ActionReceipt>("create_diagnostic_bundle"),
    getSettings: () => command<DesktopSettings>("settings_query"),
    patchSettings: (patch: SettingsPatch) =>
      command<DesktopSettings>("patch_settings", { patch }),
    patchDesktopPreferences: (patch) =>
      command<DesktopSettings>("patch_desktop_preferences", { patch }),
    checkForUpdates: () => command<UpdateStatus>("check_for_updates"),
  };
}

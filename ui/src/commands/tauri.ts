import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { SettingsPatch } from "../generated/settings.js";
import type {
  ActionReceipt,
  BootstrapResponse,
  BackupInfo,
  BackupRestorePreview,
  BackupRestoreReceipt,
  BundlePreview,
  CommandClient,
  CompleteOnboardingRequest,
  CaptureSetupPreview,
  CaptureSetupReceipt,
  DashboardResponse,
  DeletionPreview,
  DeletionReceipt,
  DesktopCapabilities,
  DesktopSettings,
  DoctorReport,
  ProxyStatus,
  HealthResponse,
  InventoryDocument,
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

import { localMidnight } from "../domain/sessionSearch.js";

function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(name, args);
}

/** The production adapter exposes only named application use cases over Tauri IPC. */
export function createTauriCommandClient(): CommandClient {
  return {
    getCapabilities: () => command<DesktopCapabilities>("desktop_capabilities"),
    getBootstrap: () => command<BootstrapResponse>("desktop_bootstrap"),
    getOnboarding: () => command<OnboardingResponse>("onboarding_status"),
    previewCaptureSetup: (harness, operation) =>
      command<CaptureSetupPreview>("preview_capture_setup", {
        harness,
        operation,
      }),
    applyCaptureSetup: (previewToken) =>
      command<CaptureSetupReceipt>("apply_capture_setup", { previewToken }),
    completeOnboarding: (request: CompleteOnboardingRequest) =>
      command<ActionReceipt>("complete_onboarding", { request }),
    getDashboard: () => command<DashboardResponse>("dashboard_query"),
    searchSessions: (filters: SessionFilters) =>
      command<SessionSearchResponse>("search_sessions", {
        filters: {
          ...filters,
          todayStart:
            filters.dateRange === "today" ? localMidnight(new Date()) : null,
        },
      }),
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
    createBackup: (destination) =>
      command<BackupInfo>("create_backup", {
        destination: destination ?? null,
      }),
    listBackups: () => command<readonly BackupInfo[]>("list_backups"),
    previewBackupRestore: (path) =>
      command<BackupRestorePreview>("preview_backup_restore", { path }),
    restoreBackup: (previewToken) =>
      command<BackupRestoreReceipt>("restore_backup", { previewToken }),
    getInventory: () => command<InventoryResponse>("inventory_query"),
    getInventoryDocument: (itemId) =>
      command<InventoryDocument>("inventory_document", { itemId }),
    saveInventoryDocument: (itemId, revision, content) =>
      command<ActionReceipt>("save_inventory_document", {
        itemId,
        revision,
        content,
      }),
    removeInventoryItem: (itemId, revision) =>
      command<ActionReceipt>("remove_inventory_item", { itemId, revision }),
    installInventoryItem: (itemId, revision, harness) =>
      command<ActionReceipt>("install_inventory_item", {
        itemId,
        revision,
        harness,
      }),
    setInventoryItemEnabled: (itemId, revision, enabled) =>
      command<ActionReceipt>("set_inventory_item_enabled", {
        itemId,
        revision,
        enabled,
      }),
    onHistoryImported: (listener) => {
      let stop: (() => void) | null = null;
      let stopped = false;
      // A window without event permission simply never refreshes on its own.
      listen("history-imported", () => listener())
        .then((unlisten) => {
          if (stopped) unlisten();
          else stop = unlisten;
        })
        .catch(() => undefined);
      return () => {
        stopped = true;
        stop?.();
      };
    },
    getPluginVerification: (itemId: string) =>
      command<PluginVerification>("plugin_verification", { itemId }),
    getProxyStatus: () => command<ProxyStatus>("proxy_status"),
    previewProxy: () => command<ProxyPreview>("preview_proxy_consent"),
    setProxyEnabled: (enabled: boolean, consentToken: string | null) =>
      command<ProxyStatus>("set_proxy_enabled", { enabled, consentToken }),
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

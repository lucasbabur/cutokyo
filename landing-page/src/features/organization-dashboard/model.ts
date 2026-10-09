"use client";

import { useAccessToken, useAuth } from "@workos-inc/authkit-nextjs/components";
import { useCallback, useEffect, useState } from "react";

import type {
  OrganizationAdminCommandResult,
  OrganizationOperationPageCommandResult,
  OrganizationSessionDetailCommandResult,
  OrganizationTelemetryFilter,
  TelemetryDimensionCommandResult,
} from "@/infrastructure/fastapi/commands";
import {
  deleteOrganizationGroupCommand,
  exportOrganizationDataCommand,
  createWorkosOrganizationCommand,
  inviteOrganizationMemberCommand,
  installManagedPluginCommand,
  loadOrganizationAdminCommand,
  loadOrganizationOperationsCommand,
  loadOrganizationSessionCommand,
  openIdentitySetupCommand,
  saveOrganizationGroupCommand,
  testManagedPluginCommand,
  uninstallManagedPluginCommand,
  updateManagedPluginCommand,
  updateOrganizationPolicyCommand,
} from "@/infrastructure/fastapi/commands";
import { tokenSavingEnabled } from "@/shared/config/features";

import type { FormEvent } from "react";

export type PolicyCapability =
  | "api-key-redaction"
  | "email-redaction"
  | "file-path-redaction"
  | "ip-address-redaction"
  | "optimization"
  | "payment-card-redaction"
  | "phone-redaction"
  | "redacted-previews";
export type OrganizationTelemetry = OrganizationAdminCommandResult;
export type OrganizationOperationPage = OrganizationOperationPageCommandResult;
export type OrganizationSessionDetail = OrganizationSessionDetailCommandResult;
export type TelemetryDimension = TelemetryDimensionCommandResult;
export type TelemetryScope =
  | { kind: "all"; label: string }
  | { id: string; kind: "group" | "user"; label: string };
export type ReportRange = "24h" | "7d" | "30d" | "90d" | "all";
export type ReportFilters = {
  harness: string;
  model: string;
  provider: string;
  range: ReportRange;
};

type TelemetryScopeUpdate = TelemetryScope | ((current: TelemetryScope) => TelemetryScope);

type LoadedOrganizationTelemetry = {
  organizationId: string;
  value: OrganizationTelemetry;
};

type LoadedOrganizationOperations = {
  organizationId: string;
  value: OrganizationOperationPageCommandResult;
};

type OrganizationError = {
  organizationId: string | undefined;
  value: string;
};

const ALL_TELEMETRY_SCOPE: TelemetryScope = { kind: "all", label: "Everyone" };
const DEFAULT_REPORT_FILTERS: ReportFilters = {
  harness: "all",
  model: "all",
  provider: "all",
  range: "30d",
};

export async function openOrganizationIdentitySetup(intent: "dsync" | "sso") {
  const link = await openIdentitySetupCommand(intent);
  window.location.assign(link);
}

export function useOrganizationOnboarding(onCreated: (organizationId: string) => Promise<unknown>) {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setPending(true);
    setError(null);
    const form = new FormData(event.currentTarget);
    try {
      const result = await createWorkosOrganizationCommand({
        domain: form.get("domain"),
        name: form.get("name"),
      });
      const activation = await onCreated(result.organization.id);
      if (activation && typeof activation === "object" && "error" in activation) {
        throw new Error(String(activation.error));
      }
      window.location.reload();
    } catch (creationError) {
      setError(
        creationError instanceof Error ? creationError.message : "Organization creation failed.",
      );
    } finally {
      setPending(false);
    }
  };
  return { error, pending, submit };
}

// eslint-disable-next-line local/no-complex-business-logic -- Coordinates authenticated API state; investigation filters and policy decisions remain server-side.
export function useOrganizationDashboard() {
  const auth = useAuth();
  const token = useAccessToken();
  const organizationId = auth.organizationId;
  const [loadedData, setLoadedData] = useState<LoadedOrganizationTelemetry | null>(null);
  const [storedError, setStoredError] = useState<OrganizationError | null>(null);
  const [savingPolicy, setSavingPolicy] = useState(false);
  const [savingGroup, setSavingGroup] = useState(false);
  const [invitingMember, setInvitingMember] = useState(false);
  const [reportFilters, setReportFilters] = useState<ReportFilters>(DEFAULT_REPORT_FILTERS);
  const [savingPlugin, setSavingPlugin] = useState(false);
  const [operationCorrelation, setOperationCorrelation] = useState("");
  const [loadedOperationPage, setLoadedOperationPage] =
    useState<LoadedOrganizationOperations | null>(null);
  const [operationLoading, setOperationLoading] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [sessionDetail, setSessionDetail] = useState<OrganizationSessionDetailCommandResult | null>(
    null,
  );
  const [sessionLoading, setSessionLoading] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [storedTelemetryScope, setStoredTelemetryScope] = useState({
    organizationId,
    value: ALL_TELEMETRY_SCOPE,
  });
  const telemetryScope =
    storedTelemetryScope.organizationId === organizationId
      ? storedTelemetryScope.value
      : ALL_TELEMETRY_SCOPE;
  const setTelemetryScope = useCallback(
    (next: TelemetryScopeUpdate) => {
      setStoredTelemetryScope((current) => {
        const active =
          current.organizationId === organizationId ? current.value : ALL_TELEMETRY_SCOPE;
        return {
          organizationId,
          value: typeof next === "function" ? next(active) : next,
        };
      });
    },
    [organizationId],
  );
  const error =
    storedError && storedError.organizationId === organizationId ? storedError.value : null;
  const setError = useCallback(
    (value: string | null) => {
      setStoredError(value === null ? null : { organizationId, value });
    },
    [organizationId],
  );
  const data =
    organizationId && loadedData?.organizationId === organizationId ? loadedData.value : null;
  const operationPage =
    organizationId && loadedOperationPage?.organizationId === organizationId
      ? loadedOperationPage.value
      : null;
  const operationAccessToken = token.accessToken;
  const getOperationAccessToken = token.getAccessToken;

  const loadOperations = useCallback(
    // eslint-disable-next-line local/no-complex-business-logic -- Coordinates one tenant-scoped cursor request; matching and bounds remain server-side.
    async (correlation: string, cursor?: number) => {
      if (!organizationId) return;
      setOperationLoading(true);
      setOperationError(null);
      const normalized = correlation.trim();
      try {
        const accessToken = operationAccessToken ?? (await getOperationAccessToken());
        if (!accessToken) throw new Error("Organization access token is unavailable");
        const page = await loadOrganizationOperationsCommand(organizationId, accessToken, {
          ...(normalized ? { correlationId: normalized } : {}),
          ...(cursor !== undefined ? { cursor } : {}),
          limit: 50,
        });
        setOperationCorrelation(normalized);
        setLoadedOperationPage((current) => ({
          organizationId,
          value: {
            nextCursor: page.nextCursor ?? null,
            operations:
              cursor !== undefined && current?.organizationId === organizationId
                ? [...current.value.operations, ...page.operations]
                : page.operations,
          },
        }));
      } catch (loadError) {
        setOperationError(
          loadError instanceof Error ? loadError.message : "Operations unavailable",
        );
      } finally {
        setOperationLoading(false);
      }
    },
    [getOperationAccessToken, operationAccessToken, organizationId],
  );

  useEffect(() => {
    if (!organizationId || !token.accessToken) return;
    const controller = new AbortController();
    void loadOrganizationAdminCommand(
      organizationId,
      token.accessToken,
      controller.signal,
      auth.permissions ?? [],
      buildTelemetryFilter(telemetryScope, reportFilters),
    )
      .then((next) => {
        setLoadedData({ organizationId, value: next });
        setError(null);
      })
      .catch((loadError: unknown) => {
        if (loadError instanceof DOMException && loadError.name === "AbortError") return;
        setError(loadError instanceof Error ? loadError.message : "Telemetry unavailable");
      });
    return () => controller.abort();
  }, [
    auth.permissions,
    organizationId,
    reportFilters,
    setError,
    telemetryScope,
    token.accessToken,
  ]);

  useEffect(() => {
    if (!organizationId || !token.accessToken || !auth.permissions?.includes("telemetry:read")) {
      return;
    }
    void loadOperations("");
  }, [auth.permissions, loadOperations, organizationId, token.accessToken]);

  useEffect(() => {
    const sessions = data?.sessions ?? [];
    if (!sessions.length) {
      setSelectedSessionId(null);
      setSessionDetail(null);
      return;
    }
    if (!selectedSessionId || !sessions.some((session) => session.id === selectedSessionId)) {
      setSelectedSessionId(sessions[0]?.id ?? null);
    }
  }, [data?.sessions, selectedSessionId]);

  useEffect(() => {
    if (!organizationId || !selectedSessionId || !token.accessToken) return;
    const controller = new AbortController();
    setSessionLoading(true);
    setSessionError(null);
    void loadOrganizationSessionCommand(
      organizationId,
      selectedSessionId,
      token.accessToken,
      controller.signal,
    )
      .then(setSessionDetail)
      .catch((detailError: unknown) => {
        if (detailError instanceof DOMException && detailError.name === "AbortError") return;
        setSessionDetail(null);
        setSessionError(
          detailError instanceof Error ? detailError.message : "Session detail unavailable",
        );
      })
      .finally(() => {
        if (!controller.signal.aborted) setSessionLoading(false);
      });
    return () => controller.abort();
  }, [organizationId, selectedSessionId, token.accessToken]);

  const setPolicyCapability = useCallback(
    // eslint-disable-next-line local/no-complex-business-logic -- Coordinates one authenticated FastAPI policy command; policy decisions remain server-side.
    async (capability: PolicyCapability, enabled: boolean) => {
      if (!organizationId || !data?.policy) return;
      if (capability === "optimization" && !tokenSavingEnabled) return;
      setSavingPolicy(true);
      setError(null);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        const policy = structuredClone(data.policy.policy);
        if (capability === "optimization") {
          policy.optimization.enabled = enabled;
          policy.optimization.providerMutation = enabled;
        }
        if (capability === "api-key-redaction") policy.redaction.apiKeys = enabled;
        if (capability === "payment-card-redaction") policy.redaction.paymentCards = enabled;
        if (capability === "email-redaction") policy.redaction.emails = enabled;
        if (capability === "phone-redaction") policy.redaction.phoneNumbers = enabled;
        if (capability === "ip-address-redaction") policy.redaction.ipAddresses = enabled;
        if (capability === "file-path-redaction") policy.redaction.filePaths = enabled;
        if (capability === "redacted-previews") policy.content.redactedPreviews = enabled;
        const updated = await updateOrganizationPolicyCommand(organizationId, accessToken, policy);
        setLoadedData((current) =>
          current?.organizationId === organizationId
            ? { ...current, value: { ...current.value, policy: updated } }
            : current,
        );
      } catch (updateError) {
        setError(updateError instanceof Error ? updateError.message : "Policy update failed");
      } finally {
        setSavingPolicy(false);
      }
    },
    [data, organizationId, setError, token],
  );

  const saveGroup = useCallback(
    async (input: { id?: string; memberIds: string[]; name: string }) => {
      if (!organizationId) return;
      setSavingGroup(true);
      setError(null);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        const saved = await saveOrganizationGroupCommand(organizationId, accessToken, input);
        setLoadedData((current) => {
          if (current?.organizationId !== organizationId) return current;
          const groups = current.value.groups.filter((group) => group.id !== saved.id);
          groups.push(saved);
          groups.sort((left, right) => left.name.localeCompare(right.name));
          return { ...current, value: { ...current.value, groups } };
        });
      } catch (saveError) {
        setError(saveError instanceof Error ? saveError.message : "Group update failed");
        throw saveError;
      } finally {
        setSavingGroup(false);
      }
    },
    [organizationId, setError, token],
  );

  const deleteGroup = useCallback(
    async (groupId: string) => {
      if (!organizationId) return;
      setSavingGroup(true);
      setError(null);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        await deleteOrganizationGroupCommand(organizationId, accessToken, groupId);
        setLoadedData((current) =>
          current?.organizationId === organizationId
            ? {
                ...current,
                value: {
                  ...current.value,
                  groups: current.value.groups.filter((group) => group.id !== groupId),
                },
              }
            : current,
        );
        setTelemetryScope((current) =>
          current.kind === "group" && current.id === groupId
            ? { kind: "all", label: "Everyone" }
            : current,
        );
      } catch (deleteError) {
        setError(deleteError instanceof Error ? deleteError.message : "Group deletion failed");
        throw deleteError;
      } finally {
        setSavingGroup(false);
      }
    },
    [organizationId, setError, setTelemetryScope, token],
  );

  const inviteMember = useCallback(
    async (input: { email: string; role: string }) => {
      if (!organizationId) return;
      setInvitingMember(true);
      setError(null);
      try {
        const invitation = await inviteOrganizationMemberCommand(input);
        setLoadedData((current) => {
          if (current?.organizationId !== organizationId) return current;
          const invitations = current.value.invitations.filter(
            (candidate) => candidate.id !== invitation.id && candidate.email !== invitation.email,
          );
          invitations.unshift(invitation);
          return { ...current, value: { ...current.value, invitations } };
        });
      } catch (inviteError) {
        setError(inviteError instanceof Error ? inviteError.message : "Invitation failed");
        throw inviteError;
      } finally {
        setInvitingMember(false);
      }
    },
    [organizationId, setError],
  );

  const saveManagedPlugin = useCallback(
    // eslint-disable-next-line local/no-complex-business-logic -- Coordinates one authenticated plugin mutation and reconciles the returned installation.
    async (input: {
      enabled: boolean;
      endpoint: string;
      id?: string;
      name: string;
      replaceSecret?: boolean;
      secret?: string;
    }) => {
      if (!organizationId) return;
      setSavingPlugin(true);
      setError(null);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        const saved = input.id
          ? await updateManagedPluginCommand(organizationId, input.id, accessToken, input)
          : await installManagedPluginCommand(organizationId, accessToken, input);
        setLoadedData((current) => {
          if (current?.organizationId !== organizationId) return current;
          const plugins = current.value.plugins.filter((plugin) => plugin.id !== saved.id);
          plugins.push(saved);
          return { ...current, value: { ...current.value, plugins } };
        });
      } catch (pluginError) {
        setError(pluginError instanceof Error ? pluginError.message : "Plugin update failed");
        throw pluginError;
      } finally {
        setSavingPlugin(false);
      }
    },
    [organizationId, setError, token],
  );

  const uninstallManagedPlugin = useCallback(
    async (pluginId: string) => {
      if (!organizationId) return;
      setSavingPlugin(true);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        await uninstallManagedPluginCommand(organizationId, pluginId, accessToken);
        setLoadedData((current) =>
          current?.organizationId === organizationId
            ? {
                ...current,
                value: {
                  ...current.value,
                  plugins: current.value.plugins.filter((plugin) => plugin.id !== pluginId),
                },
              }
            : current,
        );
      } finally {
        setSavingPlugin(false);
      }
    },
    [organizationId, token],
  );

  const testManagedPlugin = useCallback(
    async (pluginId: string) => {
      if (!organizationId) return;
      setSavingPlugin(true);
      try {
        const accessToken = await token.getAccessToken();
        if (!accessToken) throw new Error("Organization access token is unavailable");
        await testManagedPluginCommand(organizationId, pluginId, accessToken);
      } finally {
        setSavingPlugin(false);
      }
    },
    [organizationId, token],
  );

  const exportOrganizationData = useCallback(async () => {
    if (!organizationId) return;
    const accessToken = await token.getAccessToken();
    if (!accessToken) throw new Error("Organization access token is unavailable");
    const exported = await exportOrganizationDataCommand(organizationId, accessToken);
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(exported, null, 2)], { type: "application/json" }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = `cutokyo-${organizationId}-export.json`;
    link.click();
    URL.revokeObjectURL(url);
  }, [organizationId, token]);

  return {
    data,
    deleteGroup,
    error: error ?? (token.error ? String(token.error) : null),
    exportOrganizationData,
    inviteMember,
    invitingMember,
    loading: auth.loading || token.loading || (Boolean(organizationId) && !data && !error),
    loadOperations,
    operationCorrelation,
    operationError,
    operationLoading,
    operationPage,
    organizationId,
    permissions: auth.permissions ?? [],
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
    switchToOrganization: auth.switchToOrganization,
    testManagedPlugin,
    telemetryScope,
    uninstallManagedPlugin,
    user: auth.user,
  };
}

// eslint-disable-next-line local/no-complex-business-logic -- Translates four explicit report controls into the backend's tenant-safe query contract.
function buildTelemetryFilter(
  scope: TelemetryScope,
  filters: ReportFilters,
): OrganizationTelemetryFilter {
  const query: OrganizationTelemetryFilter = {
    interval: filters.range === "24h" ? "hour" : "day",
  };
  if (scope.kind === "group") query.groupId = scope.id;
  if (scope.kind === "user") query.userId = scope.id;
  if (filters.harness !== "all") query.harness = filters.harness;
  if (filters.provider !== "all") query.provider = filters.provider;
  if (filters.model !== "all") query.model = filters.model;
  const rangeDays: Partial<Record<ReportRange, number>> = {
    "24h": 1,
    "7d": 7,
    "30d": 30,
    "90d": 90,
  };
  const days = rangeDays[filters.range];
  if (days) query.startAt = new Date(Date.now() - days * 86_400_000).toISOString();
  return query;
}

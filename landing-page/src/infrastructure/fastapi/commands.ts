import type { components } from "@/generated/fastapi/schema";
import { env } from "@/shared/config/env";
import { readBoundedJson } from "@/shared/http/bounded-json";

export type AnalyzeCutokyoContextCommandInput =
  components["schemas"]["CutokyoContextAnalysisRequest"];

export type AnalyzeCutokyoContextCommandResult =
  components["schemas"]["CutokyoContextAnalysisResponse"];

export type TelemetryDimensionCommandResult = {
  costNanosUsd: number;
  events: number;
  name: string;
  tokens: number;
};

export type OrganizationOperationCommandResult = components["schemas"]["TelemetryEventResponse"];
export type OrganizationOperationPageCommandResult =
  components["schemas"]["OrganizationOperationListResponse"];

type OrganizationOperationFilter = {
  correlationId?: string;
  cursor?: number;
  limit?: number;
  requestId?: string;
  traceId?: string;
};

export type OrganizationTelemetryCommandResult = {
  byDevice: TelemetryDimensionCommandResult[];
  byGroup: TelemetryDimensionCommandResult[];
  byHarness: TelemetryDimensionCommandResult[];
  byModel: TelemetryDimensionCommandResult[];
  byProvider: TelemetryDimensionCommandResult[];
  byUser: TelemetryDimensionCommandResult[];
  cacheReadTokens: number;
  context: components["schemas"]["AttributedTokenCategories"];
  errorEvents: number;
  estimatedTokensSaved: number;
  recentEvents: OrganizationOperationCommandResult[];
  totalCostNanosUsd: number;
  totalEvents: number;
  totalRedactions: number;
  totalTokens: number;
  timeline: {
    costNanosUsd: number;
    errors: number;
    estimatedTokensSaved: number;
    events: number;
    timestamp: string;
    tokens: number;
  }[];
};

export type OrganizationTelemetryFilter = {
  deviceId?: string;
  endAt?: string;
  groupId?: string;
  harness?: string;
  interval?: "day" | "hour";
  model?: string;
  project?: string;
  provider?: string;
  startAt?: string;
  userId?: string;
  workspace?: string;
};

export type OrganizationGroupCommandResult = {
  createdAt: string;
  id: string;
  memberIds: string[];
  name: string;
  organizationId: string;
  updatedAt: string;
  updatedBy: string;
};

export type OrganizationPolicyCommandResult = {
  organizationId: string;
  policyVersion: number;
  updatedAt: string;
  updatedBy: string;
  policy: {
    content: { rawCapture: boolean; redactedPreviews: boolean; retentionDays: number };
    observability: { enabled: boolean; otlpExport: boolean };
    optimization: {
      enabled: boolean;
      failMode: "closed" | "open";
      providerMutation: boolean;
      shadowMode: boolean;
    };
    redaction: {
      apiKeys: boolean;
      emails: boolean;
      failMode: "closed" | "open";
      filePaths: boolean;
      ipAddresses: boolean;
      paymentCards: boolean;
      phoneNumbers: boolean;
    };
  };
};

export type OrganizationAdminCommandResult = OrganizationTelemetryCommandResult & {
  license: LicenseStatusCommandResult;
  auditEvents: {
    action: string;
    actorId: string;
    createdAt: string;
    detail: Record<string, unknown>;
    id: number;
    targetId: string;
  }[];
  devices: {
    appVersion: string;
    createdAt: string;
    credentialExpiresAt: string;
    id: string;
    lastSeenAt?: string | null;
    name: string;
    organizationId: string;
    platform: string;
    revokedAt?: string | null;
    userId: string;
  }[];
  groups: OrganizationGroupCommandResult[];
  invitations: OrganizationInvitationCommandResult[];
  members: OrganizationMemberCommandResult[];
  organization: { id: string; name: string };
  plugins: ManagedPluginCommandResult[];
  policy: OrganizationPolicyCommandResult | null;
  sessions: OrganizationSessionSummaryCommandResult[];
};

export type LicenseStatusCommandResult = {
  customerId?: string | null;
  customerName?: string | null;
  detail: string;
  expiresAt?: string | null;
  features: string[];
  graceUntil?: string | null;
  installationId?: string | null;
  lastRefreshedAt?: string | null;
  limits: Record<string, number>;
  mode: "disabled" | "enforced" | "shadow";
  organizationId?: string | null;
  readOnly: boolean;
  state: "active" | "disabled" | "grace" | "revoked" | "suspended" | "unavailable";
  wouldBlock: boolean;
};

type OrganizationExportCommandResult = components["schemas"]["OrganizationExportResponse"];

export type ManagedPluginCommandResult = {
  catalogId: "webhook-logs";
  createdAt: string;
  deliveredEvents: number;
  enabled: boolean;
  endpoint: string;
  hasSecret: boolean;
  id: string;
  lastAttemptAt?: string | null;
  lastError?: string | null;
  lastSuccessAt?: string | null;
  name: string;
  updatedAt: string;
};

export type OrganizationMemberCommandResult = {
  email: string;
  firstName: string | null;
  id: string;
  lastName: string | null;
  name: string;
  profilePictureUrl?: string | null;
  roles: string[];
};

export type OrganizationInvitationCommandResult = {
  email: string;
  expiresAt: string;
  id: string;
  role: string | null;
  state: "accepted" | "expired" | "pending" | "revoked";
};

export type OrganizationSessionSummaryCommandResult =
  components["schemas"]["OrganizationSessionSummary"];
export type OrganizationSessionDetailCommandResult =
  components["schemas"]["OrganizationSessionDetail"];

type WorkosOrganizationCommandResult = {
  domain: unknown;
  organization: { id: string; name: string };
};

export async function loadWorkosWidgetTokenCommand(scope: string): Promise<string> {
  const response = await fetch(`/api/workos/widgets/token?scope=${encodeURIComponent(scope)}`, {
    headers: { accept: "application/json" },
  });
  const body = await readBoundedJson<{
    message?: unknown;
    token?: unknown;
  } | null>(response, "Identity response").catch(() => null);
  if (!response.ok || typeof body?.token !== "string") {
    throw new Error(
      typeof body?.message === "string" ? body.message : "Identity authorization failed.",
    );
  }
  return body.token;
}

export async function createWorkosOrganizationCommand(input: {
  domain: FormDataEntryValue | null;
  name: FormDataEntryValue | null;
}): Promise<WorkosOrganizationCommandResult> {
  const response = await fetch("/api/workos/organizations", {
    body: JSON.stringify(input),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  const body = await readBoundedJson<{
    domain?: unknown;
    message?: unknown;
    organization?: { id?: unknown; name?: unknown };
  } | null>(response, "Organization response").catch(() => null);
  if (
    !response.ok ||
    typeof body?.organization?.id !== "string" ||
    typeof body.organization.name !== "string"
  ) {
    throw new Error(
      typeof body?.message === "string" ? body.message : "Organization creation failed.",
    );
  }
  return {
    domain: body.domain,
    organization: { id: body.organization.id, name: body.organization.name },
  };
}

export async function analyzeCutokyoContextCommand(
  input: AnalyzeCutokyoContextCommandInput,
  signal?: AbortSignal,
): Promise<AnalyzeCutokyoContextCommandResult> {
  const init: RequestInit = {
    body: JSON.stringify(input),
    headers: { "Content-Type": "application/json" },
    method: "POST",
  };
  if (signal) init.signal = signal;

  const response = await fetch(`${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/context/analyze`, init);

  if (!response.ok) {
    throw new Error(`Cutokyo analysis failed with ${response.status}`);
  }

  return readBoundedJson<AnalyzeCutokyoContextCommandResult>(response, "FastAPI response");
}

async function loadOrganizationTelemetryCommand(
  organizationId: string,
  accessToken: string,
  signal: AbortSignal,
  filter?: OrganizationTelemetryFilter,
): Promise<OrganizationTelemetryCommandResult> {
  const query = new URLSearchParams();
  if (filter?.startAt) query.set("from", filter.startAt);
  if (filter?.endAt) query.set("to", filter.endAt);
  if (filter?.interval) query.set("interval", filter.interval);
  if (filter?.harness) query.set("harness", filter.harness);
  if (filter?.provider) query.set("provider", filter.provider);
  if (filter?.model) query.set("model", filter.model);
  if (filter?.groupId) query.set("groupId", filter.groupId);
  if (filter?.userId) query.set("userId", filter.userId);
  if (filter?.deviceId) query.set("deviceId", filter.deviceId);
  if (filter?.workspace) query.set("workspace", filter.workspace);
  if (filter?.project) query.set("project", filter.project);
  query.set("limit", "500");
  const suffix = query.size ? `?${query.toString()}` : "";
  const response = await fetch(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/telemetry/summary${suffix}`,
    {
      headers: { Authorization: `Bearer ${accessToken}` },
      signal,
    },
  );
  if (!response.ok) {
    const body = await readBoundedJson<{ detail?: unknown } | null>(
      response,
      "FastAPI response",
    ).catch(() => null);
    throw new Error(typeof body?.detail === "string" ? body.detail : `HTTP ${response.status}`);
  }
  return readBoundedJson<OrganizationTelemetryCommandResult>(response, "FastAPI response");
}

export async function loadOrganizationAdminCommand(
  organizationId: string,
  accessToken: string,
  signal: AbortSignal,
  permissions: string[],
  filter?: OrganizationTelemetryFilter,
): Promise<OrganizationAdminCommandResult> {
  const root = `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}`;
  const [telemetry, devices, groups, directory, policy, audit, sessions, plugins, license] =
    await Promise.all([
      loadOrganizationTelemetryCommand(organizationId, accessToken, signal, filter),
      permissions.includes("devices:read")
        ? authorizedJson<{ devices: OrganizationAdminCommandResult["devices"] }>(
            `${root}/devices`,
            accessToken,
            signal,
          )
        : Promise.resolve({ devices: [] }),
      permissions.includes("groups:read")
        ? authorizedJson<{ groups: OrganizationGroupCommandResult[] }>(
            `${root}/groups`,
            accessToken,
            signal,
          )
        : Promise.resolve({ groups: [] }),
      permissions.includes("telemetry:read")
        ? loadOrganizationMembersCommand(signal)
        : Promise.resolve({
            invitations: [],
            members: [],
            organization: { id: organizationId, name: "Organization" },
          }),
      permissions.includes("policies:read")
        ? authorizedJson<OrganizationPolicyCommandResult>(`${root}/policy`, accessToken, signal)
        : Promise.resolve(null),
      permissions.includes("audit:read")
        ? authorizedJson<{ events: OrganizationAdminCommandResult["auditEvents"] }>(
            `${root}/audit`,
            accessToken,
            signal,
          )
        : Promise.resolve({ events: [] }),
      permissions.includes("sessions:read")
        ? authorizedJson<{ sessions: OrganizationSessionSummaryCommandResult[] }>(
            `${root}/sessions${telemetryFilterQuery(filter)}`,
            accessToken,
            signal,
          )
        : Promise.resolve({ sessions: [] }),
      permissions.includes("policies:read")
        ? authorizedJson<{ installations: ManagedPluginCommandResult[] }>(
            `${root}/plugins`,
            accessToken,
            signal,
          )
        : Promise.resolve({ installations: [] }),
      authorizedJson<LicenseStatusCommandResult>(
        `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/license/status`,
        accessToken,
        signal,
      ),
    ]);
  return {
    ...telemetry,
    auditEvents: audit.events,
    devices: devices.devices,
    groups: groups.groups,
    invitations: directory.invitations,
    license,
    members: directory.members,
    organization: directory.organization,
    policy,
    plugins: plugins.installations,
    sessions: sessions.sessions,
  };
}

export function exportOrganizationDataCommand(
  organizationId: string,
  accessToken: string,
): Promise<OrganizationExportCommandResult> {
  return authorizedJson<OrganizationExportCommandResult>(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/export`,
    accessToken,
    new AbortController().signal,
  );
}

export async function installManagedPluginCommand(
  organizationId: string,
  accessToken: string,
  input: { enabled: boolean; endpoint: string; name: string; secret?: string },
): Promise<ManagedPluginCommandResult> {
  return authorizedJson<ManagedPluginCommandResult>(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/plugins`,
    accessToken,
    undefined,
    {
      body: JSON.stringify({ catalogId: "webhook-logs", ...input }),
      headers: { "Content-Type": "application/json" },
      method: "POST",
    },
  );
}

export async function updateManagedPluginCommand(
  organizationId: string,
  pluginId: string,
  accessToken: string,
  input: {
    enabled: boolean;
    endpoint: string;
    name: string;
    replaceSecret?: boolean;
    secret?: string;
  },
): Promise<ManagedPluginCommandResult> {
  return authorizedJson<ManagedPluginCommandResult>(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/plugins/${encodeURIComponent(pluginId)}`,
    accessToken,
    undefined,
    {
      body: JSON.stringify({ catalogId: "webhook-logs", ...input }),
      headers: { "Content-Type": "application/json" },
      method: "PUT",
    },
  );
}

export async function uninstallManagedPluginCommand(
  organizationId: string,
  pluginId: string,
  accessToken: string,
): Promise<void> {
  await authorizedJson(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/plugins/${encodeURIComponent(pluginId)}`,
    accessToken,
    undefined,
    { method: "DELETE" },
  );
}

export async function testManagedPluginCommand(
  organizationId: string,
  pluginId: string,
  accessToken: string,
): Promise<void> {
  await authorizedJson(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/plugins/${encodeURIComponent(pluginId)}/test`,
    accessToken,
    undefined,
    { body: "{}", method: "POST" },
  );
}

async function loadOrganizationMembersCommand(signal: AbortSignal): Promise<{
  invitations: OrganizationInvitationCommandResult[];
  members: OrganizationMemberCommandResult[];
  organization: { id: string; name: string };
}> {
  const response = await fetch("/api/workos/members", { signal });
  const body = await readBoundedJson<{
    members?: OrganizationMemberCommandResult[];
    invitations?: OrganizationInvitationCommandResult[];
    message?: unknown;
    organization?: { id?: unknown; name?: unknown };
  } | null>(response, "Organization directory response").catch(() => null);
  if (
    !response.ok ||
    !Array.isArray(body?.members) ||
    !Array.isArray(body.invitations) ||
    typeof body.organization?.id !== "string" ||
    typeof body.organization.name !== "string"
  ) {
    throw new Error(
      typeof body?.message === "string" ? body.message : "Organization directory lookup failed.",
    );
  }
  return {
    invitations: body.invitations,
    members: body.members,
    organization: { id: body.organization.id, name: body.organization.name },
  };
}

export async function inviteOrganizationMemberCommand(input: {
  email: string;
  role: string;
}): Promise<OrganizationInvitationCommandResult> {
  const response = await fetch("/api/workos/members", {
    body: JSON.stringify(input),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  const body = await readBoundedJson<{
    invitation?: OrganizationInvitationCommandResult;
    message?: unknown;
  } | null>(response, "Invitation response").catch(() => null);
  if (!response.ok || !body?.invitation) {
    throw new Error(typeof body?.message === "string" ? body.message : "Invitation failed.");
  }
  return body.invitation;
}

export async function openIdentitySetupCommand(intent: "dsync" | "sso"): Promise<string> {
  const response = await fetch("/api/identity/portal", {
    body: JSON.stringify({ intent }),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  const body = await readBoundedJson<{ link?: unknown; message?: unknown } | null>(
    response,
    "Identity setup response",
  ).catch(() => null);
  if (!response.ok || typeof body?.link !== "string") {
    throw new Error(
      typeof body?.message === "string" ? body.message : "Identity setup could not be opened.",
    );
  }
  return body.link;
}

export async function loadOrganizationSessionCommand(
  organizationId: string,
  sessionId: string,
  accessToken: string,
  signal?: AbortSignal,
): Promise<OrganizationSessionDetailCommandResult> {
  return authorizedJson<OrganizationSessionDetailCommandResult>(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/sessions/${encodeURIComponent(sessionId)}`,
    accessToken,
    signal,
  );
}

export async function loadOrganizationOperationsCommand(
  organizationId: string,
  accessToken: string,
  filter: OrganizationOperationFilter = {},
  signal?: AbortSignal,
): Promise<OrganizationOperationPageCommandResult> {
  const query = new URLSearchParams();
  if (filter.correlationId) query.set("correlationId", filter.correlationId);
  if (filter.traceId) query.set("traceId", filter.traceId);
  if (filter.requestId) query.set("requestId", filter.requestId);
  if (filter.cursor !== undefined) query.set("cursor", String(filter.cursor));
  query.set("limit", String(filter.limit ?? 50));
  return authorizedJson<OrganizationOperationPageCommandResult>(
    `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/operations?${query.toString()}`,
    accessToken,
    signal,
  );
}

function telemetryFilterQuery(filter?: OrganizationTelemetryFilter): string {
  const query = new URLSearchParams();
  if (filter?.groupId) query.set("groupId", filter.groupId);
  if (filter?.userId) query.set("userId", filter.userId);
  return query.size ? `?${query.toString()}` : "";
}

export async function saveOrganizationGroupCommand(
  organizationId: string,
  accessToken: string,
  input: { id?: string; memberIds: string[]; name: string },
): Promise<OrganizationGroupCommandResult> {
  const root = `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/groups`;
  return authorizedJson<OrganizationGroupCommandResult>(
    input.id ? `${root}/${encodeURIComponent(input.id)}` : root,
    accessToken,
    undefined,
    {
      body: JSON.stringify({ memberIds: input.memberIds, name: input.name }),
      method: input.id ? "PUT" : "POST",
    },
  );
}

export async function deleteOrganizationGroupCommand(
  organizationId: string,
  accessToken: string,
  groupId: string,
): Promise<void> {
  const root = `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}/groups`;
  await authorizedJson(`${root}/${encodeURIComponent(groupId)}`, accessToken, undefined, {
    method: "DELETE",
  });
}

export async function updateOrganizationPolicyCommand(
  organizationId: string,
  accessToken: string,
  policy: OrganizationPolicyCommandResult["policy"],
): Promise<OrganizationPolicyCommandResult> {
  const root = `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/${encodeURIComponent(organizationId)}`;
  return authorizedJson<OrganizationPolicyCommandResult>(`${root}/policy`, accessToken, undefined, {
    body: JSON.stringify(policy),
    method: "PUT",
  });
}

async function authorizedJson<T>(
  url: string,
  accessToken: string,
  signal?: AbortSignal,
  init?: RequestInit,
): Promise<T> {
  const headers = new Headers(init?.headers);
  headers.set("Authorization", `Bearer ${accessToken}`);
  headers.set("Content-Type", "application/json");
  const requestInit: RequestInit = { ...init, headers };
  if (signal) requestInit.signal = signal;
  const response = await fetch(url, requestInit);
  if (!response.ok) {
    const body = await readBoundedJson<{ detail?: unknown } | null>(
      response,
      "FastAPI response",
    ).catch(() => null);
    throw new Error(typeof body?.detail === "string" ? body.detail : `HTTP ${response.status}`);
  }
  return readBoundedJson<T>(response, "FastAPI response");
}

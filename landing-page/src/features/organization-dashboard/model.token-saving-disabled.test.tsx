import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { OrganizationPolicyCommandResult } from "@/infrastructure/fastapi/commands";
import {
  loadOrganizationAdminCommand,
  updateOrganizationPolicyCommand,
} from "@/infrastructure/fastapi/commands";

import { useOrganizationDashboard } from "./model";

import type { OrganizationTelemetry } from "./model";

const workos = vi.hoisted(() => ({
  accessToken: "token-old",
  organizationId: "org-old" as string | undefined,
  permissions: [] as string[],
}));

vi.mock("@workos-inc/authkit-nextjs/components", () => ({
  useAccessToken: () => ({
    accessToken: workos.accessToken,
    error: null,
    getAccessToken: vi.fn().mockResolvedValue(workos.accessToken),
    loading: false,
  }),
  useAuth: () => ({
    loading: false,
    organizationId: workos.organizationId,
    permissions: workos.permissions,
    user: null,
  }),
}));

vi.mock("@/infrastructure/fastapi/commands", () => ({
  deleteOrganizationGroupCommand: vi.fn(),
  loadOrganizationAdminCommand: vi.fn(),
  loadOrganizationSessionCommand: vi.fn(),
  saveOrganizationGroupCommand: vi.fn(),
  updateOrganizationPolicyCommand: vi.fn(),
  exportOrganizationDataCommand: vi.fn(),
  createWorkosOrganizationCommand: vi.fn(),
  inviteOrganizationMemberCommand: vi.fn(),
  installManagedPluginCommand: vi.fn(),
  openIdentitySetupCommand: vi.fn(),
  testManagedPluginCommand: vi.fn(),
  uninstallManagedPluginCommand: vi.fn(),
  updateManagedPluginCommand: vi.fn(),
}));

vi.mock("@/shared/config/features", () => ({ tokenSavingEnabled: false }));

const policy: OrganizationPolicyCommandResult = {
  organizationId: "org-old",
  policy: {
    content: { rawCapture: false, redactedPreviews: true, retentionDays: 30 },
    observability: { enabled: true, otlpExport: true },
    optimization: {
      enabled: false,
      failMode: "open",
      providerMutation: false,
      shadowMode: false,
    },
    redaction: {
      apiKeys: true,
      emails: true,
      failMode: "closed",
      filePaths: true,
      ipAddresses: true,
      paymentCards: true,
      phoneNumbers: true,
    },
  },
  policyVersion: 1,
  updatedAt: "2026-07-12T00:00:00Z",
  updatedBy: "user-admin",
};

describe("policy writes without token saving", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    workos.organizationId = "org-old";
  });

  it("never sends an optimization policy write", async () => {
    const telemetry = { policy } as OrganizationTelemetry;
    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce(telemetry);

    const { result } = renderHook(() => useOrganizationDashboard());
    await waitFor(() => expect(result.current.data).toBe(telemetry));

    await act(() => result.current.setPolicyCapability("optimization", true));

    expect(updateOrganizationPolicyCommand).not.toHaveBeenCalled();
  });

  it("still sends unrelated redaction policy writes", async () => {
    const telemetry = { policy } as OrganizationTelemetry;
    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce(telemetry);
    vi.mocked(updateOrganizationPolicyCommand).mockResolvedValueOnce({
      ...policy,
      policy: { ...policy.policy, redaction: { ...policy.policy.redaction, emails: false } },
    });

    const { result } = renderHook(() => useOrganizationDashboard());
    await waitFor(() => expect(result.current.data).toBe(telemetry));

    await act(() => result.current.setPolicyCapability("email-redaction", false));

    expect(updateOrganizationPolicyCommand).toHaveBeenCalledWith(
      "org-old",
      "token-old",
      expect.objectContaining({ redaction: expect.objectContaining({ emails: false }) }),
    );
  });
});

import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, vi } from "vitest";

import {
  loadOrganizationAdminCommand,
  loadOrganizationSessionCommand,
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
}));

describe("useOrganizationDashboard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    workos.accessToken = "token-old";
    workos.organizationId = "org-old";
    workos.permissions = [];
  });

  it("never exposes telemetry loaded for a previous organization", async () => {
    const oldTelemetry = { totalEvents: 12 } as OrganizationTelemetry;
    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce(oldTelemetry);

    const { rerender, result } = renderHook(() => useOrganizationDashboard());

    await waitFor(() => expect(result.current.data).toBe(oldTelemetry));

    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce(oldTelemetry);
    act(() =>
      result.current.setTelemetryScope({ id: "group-old", kind: "group", label: "Old group" }),
    );
    await waitFor(() => expect(loadOrganizationAdminCommand).toHaveBeenCalledTimes(2));

    workos.organizationId = "org-new";
    vi.mocked(loadOrganizationAdminCommand).mockReturnValueOnce(new Promise(() => undefined));
    rerender();

    expect(result.current.organizationId).toBe("org-new");
    expect(result.current.data).toBeNull();
    expect(result.current.loading).toBe(true);
    expect(result.current.telemetryScope).toEqual({ kind: "all", label: "Everyone" });
    await waitFor(() => expect(loadOrganizationAdminCommand).toHaveBeenCalledTimes(3));
    expect(vi.mocked(loadOrganizationAdminCommand).mock.calls[2]?.[4]).toMatchObject({
      interval: "day",
    });
  });

  it("surfaces organization telemetry load failures", async () => {
    vi.mocked(loadOrganizationAdminCommand).mockRejectedValueOnce(
      new Error("Telemetry access denied"),
    );

    const { rerender, result } = renderHook(() => useOrganizationDashboard());

    await waitFor(() => expect(result.current.error).toBe("Telemetry access denied"));
    expect(result.current.data).toBeNull();
    expect(result.current.loading).toBe(false);

    workos.organizationId = "org-new";
    vi.mocked(loadOrganizationAdminCommand).mockReturnValueOnce(new Promise(() => undefined));
    rerender();

    expect(result.current.error).toBeNull();
    expect(result.current.loading).toBe(true);
  });

  it("updates policy state only for the loaded organization", async () => {
    const policy = {
      organizationId: "org-old",
      policy: {
        content: { rawCapture: false, redactedPreviews: true, retentionDays: 30 },
        observability: { enabled: true, otlpExport: false },
        optimization: {
          enabled: true,
          failMode: "closed" as const,
          providerMutation: true,
          shadowMode: false,
        },
        redaction: {
          apiKeys: true,
          emails: true,
          failMode: "closed" as const,
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
    const telemetry = { policy } as unknown as OrganizationTelemetry;
    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce(telemetry);
    vi.mocked(updateOrganizationPolicyCommand).mockResolvedValueOnce({
      ...policy,
      policy: { ...policy.policy, optimization: { ...policy.policy.optimization, enabled: false } },
    });

    const { result } = renderHook(() => useOrganizationDashboard());
    await waitFor(() => expect(result.current.data).toBe(telemetry));

    await act(() => result.current.setPolicyCapability("optimization", false));

    expect(updateOrganizationPolicyCommand).toHaveBeenCalledWith(
      "org-old",
      "token-old",
      expect.objectContaining({
        optimization: expect.objectContaining({ enabled: false, providerMutation: false }),
      }),
    );
    expect(result.current.data?.policy?.policy.optimization.enabled).toBe(false);
  });

  it("selects and loads the first redacted organization session", async () => {
    const summary = {
      id: "session-ada",
      lastActivityAt: "2026-07-12T00:00:00Z",
    };
    const detail = { ...summary, messages: [{ content: "[REDACTED:email]" }] };
    vi.mocked(loadOrganizationAdminCommand).mockResolvedValueOnce({
      sessions: [summary],
    } as OrganizationTelemetry);
    vi.mocked(loadOrganizationSessionCommand).mockResolvedValueOnce(detail as never);

    const { result } = renderHook(() => useOrganizationDashboard());

    await waitFor(() => expect(result.current.selectedSessionId).toBe("session-ada"));
    await waitFor(() => expect(result.current.sessionDetail).toBe(detail));
    expect(loadOrganizationSessionCommand).toHaveBeenCalledWith(
      "org-old",
      "session-ada",
      "token-old",
      expect.any(AbortSignal),
    );
  });
});

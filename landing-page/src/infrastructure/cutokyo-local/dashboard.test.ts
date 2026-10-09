import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  dashboardErrorMessage,
  MAX_LOCAL_RESPONSE_BYTES,
  useLocalDesktopDashboard,
} from "./dashboard";

import type { FeatureSettings } from "./dashboard";

const settings: FeatureSettings = {
  apiKeyRedaction: true,
  autoConnect: true,
  compression: true,
  conversationRecording: false,
  emailRedaction: true,
  filePathRedaction: true,
  ipAddressRedaction: true,
  observability: true,
  organizationSync: false,
  otlpExport: false,
  paymentCardRedaction: true,
  phoneNumberRedaction: true,
  plugins: true,
  providerMutation: true,
};

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("dashboardErrorMessage", () => {
  it("turns browser transport failures into an actionable local endpoint message", () => {
    expect(dashboardErrorMessage(new TypeError("Failed to fetch"), "Local proxy unavailable")).toBe(
      "Cutokyo proxy is unavailable at localhost:49321.",
    );
  });

  it("preserves API details and uses a fallback for unknown failures", () => {
    expect(dashboardErrorMessage(new Error("Device enrollment failed"), "Action failed")).toBe(
      "Device enrollment failed",
    );
    expect(dashboardErrorMessage({ reason: "unknown" }, "Action failed")).toBe("Action failed");
  });

  it("exposes settings update failures instead of rejecting an unattended promise", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          Response.json({ detail: "Settings are locked by policy" }, { status: 403 }),
        ),
    );
    const { result } = renderHook(() => useLocalDesktopDashboard());

    await act(() => result.current.updateSettings(settings));

    expect(result.current.error).toBe("Settings are locked by policy");
  });

  it("surfaces unsafe desktop authorization URLs as action errors", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(Response.json({ authorizationUrl: "javascript:alert(1)" })),
    );
    const { result } = renderHook(() => useLocalDesktopDashboard());

    await act(() => result.current.runAction("signin"));

    expect(result.current.action).toBeNull();
    expect(result.current.error).toBe("External URLs must use HTTP or HTTPS.");
  });

  it("stops reading an oversized chunked local response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(new Uint8Array(MAX_LOCAL_RESPONSE_BYTES + 1), {
          headers: { "content-type": "application/json" },
        }),
      ),
    );
    const { result } = renderHook(() => useLocalDesktopDashboard());

    await act(() => result.current.updateSettings(settings));

    expect(result.current.error).toBe(
      `Local proxy response exceeded ${MAX_LOCAL_RESPONSE_BYTES} bytes.`,
    );
  });

  it("does not overlap or retain a stalled desktop snapshot poll", async () => {
    const pending: {
      resolve: (response: Response) => void;
      signal: AbortSignal | null | undefined;
    }[] = [];
    vi.stubEnv("NODE_ENV", "development");
    vi.useFakeTimers();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_input: string | URL | Request, init?: RequestInit) => {
        return new Promise<Response>((resolve) => {
          pending.push({ resolve, signal: init?.signal });
        });
      }),
    );

    const { unmount } = renderHook(() => useLocalDesktopDashboard());
    await act(async () => Promise.resolve());
    expect(pending).toHaveLength(12);

    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(pending).toHaveLength(12);

    unmount();
    expect(pending.every((request) => request.signal?.aborted)).toBe(true);
    for (const request of pending) request.resolve(Response.json({}));
    await act(async () => Promise.resolve());
  });
});

import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { MAX_JSON_RESPONSE_BYTES } from "@/shared/http/bounded-json";

import { AuthKitProvider, useAccessToken, useAuth } from "./static-authkit-components";

const opener = vi.hoisted(() => ({
  openExternalUrl: vi.fn(async () => undefined),
}));

vi.mock("@/infrastructure/tauri/open-url", () => opener);

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it("rejects an oversized local desktop auth response", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockImplementation((input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/cutokyo/auth/session")) {
        return Promise.resolve(
          Response.json({ authenticated: false, authConfigured: true, user: null }),
        );
      }
      return Promise.resolve(
        new Response(new Uint8Array(MAX_JSON_RESPONSE_BYTES + 1), {
          headers: { "content-type": "application/json" },
        }),
      );
    }),
  );
  const { result } = renderHook(() => useAuth());
  await waitFor(() => expect(result.current.loading).toBe(false));

  await act(async () => {
    await expect(result.current.refreshAuth({ ensureSignedIn: true })).rejects.toThrow(
      `Local auth response exceeded ${MAX_JSON_RESPONSE_BYTES} bytes.`,
    );
  });
});

it("provides the static wrapper and empty desktop access-token contract", async () => {
  render(
    <AuthKitProvider>
      <span>desktop child</span>
    </AuthKitProvider>,
  );
  expect(screen.getByText("desktop child")).toBeTruthy();

  const { result } = renderHook(() => useAccessToken());
  expect(result.current.accessToken).toBeUndefined();
  expect(result.current.error).toBeUndefined();
  expect(result.current.loading).toBe(false);
  await expect(result.current.getAccessToken()).resolves.toBeUndefined();
  await expect(result.current.refresh()).resolves.toBeUndefined();
});

it("does not overlap stalled desktop auth polling requests", async () => {
  let sessionCalls = 0;
  let finishPolling!: (response: Response) => void;
  const stalledPolling = new Promise<Response>((resolve) => {
    finishPolling = resolve;
  });
  const fetch = vi.fn().mockImplementation((input: string | URL | Request) => {
    const url = String(input);
    if (url.endsWith("/cutokyo/auth/start")) {
      return Promise.resolve(
        Response.json({ authorizationUrl: "https://auth.example.test/start" }),
      );
    }
    sessionCalls += 1;
    if (sessionCalls <= 2) {
      return Promise.resolve(
        Response.json({ authenticated: false, authConfigured: true, user: null }),
      );
    }
    return stalledPolling;
  });
  vi.stubGlobal("fetch", fetch);
  const { result } = renderHook(() => useAuth());
  await waitFor(() => expect(result.current.loading).toBe(false));

  vi.useFakeTimers();
  await act(async () => {
    await expect(result.current.refreshAuth({ ensureSignedIn: true })).resolves.toEqual({
      user: null,
    });
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5_000);
  });

  expect(sessionCalls).toBe(3);
  finishPolling(Response.json({ authenticated: true, authConfigured: true, user: null }));
  await act(async () => Promise.resolve());
});

it("stops desktop auth polling when pending OAuth state expires", async () => {
  let sessionCalls = 0;
  const fetch = vi.fn().mockImplementation((input: string | URL | Request) => {
    const url = String(input);
    if (url.endsWith("/cutokyo/auth/start")) {
      return Promise.resolve(
        Response.json({ authorizationUrl: "https://auth.example.test/start" }),
      );
    }
    sessionCalls += 1;
    return Promise.resolve(
      Response.json({ authenticated: false, authConfigured: true, user: null }),
    );
  });
  vi.stubGlobal("fetch", fetch);
  const { result } = renderHook(() => useAuth());
  await waitFor(() => expect(result.current.loading).toBe(false));

  vi.useFakeTimers();
  await act(async () => {
    await result.current.refreshAuth({ ensureSignedIn: true });
    await vi.advanceTimersByTimeAsync(10 * 60 * 1_000 + 1);
  });
  const callsAtDeadline = sessionCalls;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });

  expect(callsAtDeadline).toBeGreaterThan(2);
  expect(sessionCalls).toBe(callsAtDeadline);
});

it("refreshes an authenticated desktop session and signs out", async () => {
  const user = { email: "operator@example.com", name: "Operator", subject: "user-123" };
  const fetch = vi.fn().mockImplementation((input: string | URL | Request) => {
    const url = String(input);
    if (url.endsWith("/cutokyo/auth/sign-out")) return Promise.resolve(Response.json({}));
    return Promise.resolve(Response.json({ authenticated: true, authConfigured: true, user }));
  });
  vi.stubGlobal("fetch", fetch);
  const { result } = renderHook(() => useAuth());
  await waitFor(() => expect(result.current.loading).toBe(false));

  await expect(result.current.refreshAuth()).resolves.toEqual({ user });
  await act(() => result.current.signOut());

  expect(result.current.user).toBeNull();
  expect(fetch.mock.calls.some(([url]) => String(url).endsWith("/cutokyo/auth/sign-out"))).toBe(
    true,
  );
});

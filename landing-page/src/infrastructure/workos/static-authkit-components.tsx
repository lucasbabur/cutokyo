"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { openExternalUrl } from "@/infrastructure/tauri/open-url";
import { readBoundedJson } from "@/shared/http/bounded-json";

type AuthKitProviderProps = {
  children: React.ReactNode;
};

type DesktopAuthUser = {
  email?: string | null;
  name?: string | null;
  subject?: string | null;
};

type DesktopAuthSession = {
  authenticated: boolean;
  authConfigured: boolean;
  user?: DesktopAuthUser | null;
};

const AUTH_POLL_INTERVAL_MS = 1_000;
const AUTH_POLL_TIMEOUT_MS = 10 * 60 * 1_000;

export function AuthKitProvider({ children }: AuthKitProviderProps) {
  return children;
}

export function useAuth() {
  const [loading, setLoading] = useState(true);
  const [user, setUser] = useState<DesktopAuthUser | null>(null);
  const pollTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pollGeneration = useRef(0);

  const loadSession = useCallback(async () => {
    const session = await fetchDesktopSession();
    setUser(session.user ?? null);
    setLoading(false);
    return session;
  }, []);

  const stopAuthPolling = useCallback(() => {
    pollGeneration.current += 1;
    if (pollTimer.current) {
      clearTimeout(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  const startAuthPolling = useCallback(() => {
    stopAuthPolling();
    const generation = pollGeneration.current;
    const deadline = Date.now() + AUTH_POLL_TIMEOUT_MS;
    const schedule = () => {
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        pollTimer.current = null;
        return;
      }
      pollTimer.current = setTimeout(
        () => {
          void fetchDesktopSession()
            .then((session) => {
              if (pollGeneration.current !== generation) return;
              setUser(session.user ?? null);
              if (session.authenticated) {
                pollTimer.current = null;
                return;
              }
              schedule();
            })
            .catch(() => {
              if (pollGeneration.current === generation) schedule();
            });
        },
        Math.min(AUTH_POLL_INTERVAL_MS, remaining),
      );
    };
    schedule();
  }, [stopAuthPolling]);

  useEffect(() => {
    let mounted = true;
    void fetchDesktopSession()
      .then((session) => {
        if (!mounted) return;
        setUser(session.user ?? null);
        setLoading(false);
      })
      .catch(() => {
        if (!mounted) return;
        setUser(null);
        setLoading(false);
      });
    return () => {
      mounted = false;
      stopAuthPolling();
    };
  }, [loadSession, stopAuthPolling]);

  const refreshAuth = useCallback(
    async ({ ensureSignedIn }: { ensureSignedIn?: boolean } = {}) => {
      const session = await loadSession();
      if (!ensureSignedIn || session.authenticated) {
        return { user: session.user ?? null };
      }

      const response = await fetch("http://localhost:49321/cutokyo/auth/start", {
        headers: { accept: "application/json" },
        method: "POST",
      });
      if (!response.ok) {
        return { error: new Error(await errorMessage(response, "Cutokyo desktop auth failed")) };
      }

      const body = await readBoundedJson<{ authorizationUrl: string }>(
        response,
        "Local auth response",
      );
      try {
        await openExternalUrl(body.authorizationUrl);
      } catch (error) {
        return {
          error: error instanceof Error ? error : new Error("Cutokyo could not open sign-in."),
        };
      }
      startAuthPolling();
      return { user: null };
    },
    [loadSession, startAuthPolling],
  );

  const signOut = useCallback(async () => {
    stopAuthPolling();
    setLoading(true);
    try {
      const response = await fetch("http://localhost:49321/cutokyo/auth/sign-out", {
        headers: { accept: "application/json" },
        method: "POST",
      });
      if (!response.ok) {
        throw new Error(`Cutokyo desktop sign out failed with ${response.status}`);
      }
      setUser(null);
    } finally {
      setLoading(false);
    }
  }, [stopAuthPolling]);

  return {
    loading,
    organizationId: null as string | null,
    permissions: [] as string[],
    refreshAuth,
    signOut,
    user,
  };
}

export function useAccessToken() {
  return {
    accessToken: undefined as string | undefined,
    error: undefined as Error | undefined,
    getAccessToken: async () => undefined as string | undefined,
    loading: false,
    refresh: async () => undefined as string | undefined,
  };
}

async function fetchDesktopSession(): Promise<DesktopAuthSession> {
  const response = await fetch("http://localhost:49321/cutokyo/auth/session", {
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`Cutokyo desktop auth session failed with ${response.status}`);
  }
  return readBoundedJson<DesktopAuthSession>(response, "Local auth response");
}

async function errorMessage(response: Response, fallback: string): Promise<string> {
  const body = await readBoundedJson<unknown>(response, "Local auth response").catch(() => null);
  if (body && typeof body === "object" && "detail" in body && typeof body.detail === "string") {
    return body.detail;
  }
  return `${fallback} with ${response.status}`;
}

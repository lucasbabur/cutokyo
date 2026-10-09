import { useSyncExternalStore } from "react";

import type { RoutePath } from "./contracts.js";

export interface AppLocation {
  readonly page: RoutePath;
  readonly sessionId: string | null;
}

const VALID_ROUTES = new Set<RoutePath>([
  "/onboarding",
  "/dashboard",
  "/sessions",
  "/inventory",
  "/health",
  "/data",
  "/settings",
]);

function hashValue(): string {
  return globalThis.location?.hash.slice(1) ?? "";
}

function subscribe(listener: () => void): () => void {
  globalThis.addEventListener?.("hashchange", listener);
  return () => globalThis.removeEventListener?.("hashchange", listener);
}

function parseLocation(hash: string): AppLocation {
  const normalized = hash.startsWith("/") ? hash : `/${hash}`;
  if (
    normalized.startsWith("/sessions/") &&
    normalized.length > "/sessions/".length
  ) {
    return {
      page: "/sessions",
      sessionId: decodeURIComponent(normalized.slice("/sessions/".length)),
    };
  }
  return {
    page: VALID_ROUTES.has(normalized as RoutePath)
      ? (normalized as RoutePath)
      : "/dashboard",
    sessionId: null,
  };
}

export function useAppLocation(): AppLocation {
  const hash = useSyncExternalStore(subscribe, hashValue, () => "/dashboard");
  return parseLocation(hash);
}

export function routeHref(path: RoutePath, sessionId?: string): string {
  return sessionId === undefined
    ? `#${path}`
    : `#${path}/${encodeURIComponent(sessionId)}`;
}

export function navigate(path: RoutePath, sessionId?: string): void {
  globalThis.location.hash = routeHref(path, sessionId).slice(1);
}

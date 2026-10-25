import { useSyncExternalStore } from "react";

import type { RoutePath } from "./contracts.js";

export interface AppLocation {
  readonly page: RoutePath;
  readonly sessionId: string | null;
  readonly toolName: string | null;
}

const VALID_ROUTES = new Set<RoutePath>([
  "/onboarding",
  "/dashboard",
  "/sessions",
  "/tools",
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
      toolName: null,
    };
  }
  if (
    normalized.startsWith("/tools/") &&
    normalized.length > "/tools/".length
  ) {
    return {
      page: "/tools",
      sessionId: null,
      toolName: decodeURIComponent(normalized.slice("/tools/".length)),
    };
  }
  return {
    page: VALID_ROUTES.has(normalized as RoutePath)
      ? (normalized as RoutePath)
      : "/dashboard",
    sessionId: null,
    toolName: null,
  };
}

export function useAppLocation(): AppLocation {
  const hash = useSyncExternalStore(subscribe, hashValue, () => "/dashboard");
  return parseLocation(hash);
}

/** `itemId` is a session ID under /sessions and a tool name under /tools. */
export function routeHref(path: RoutePath, itemId?: string): string {
  return itemId === undefined
    ? `#${path}`
    : `#${path}/${encodeURIComponent(itemId)}`;
}

export function navigate(path: RoutePath, itemId?: string): void {
  globalThis.location.hash = routeHref(path, itemId).slice(1);
}

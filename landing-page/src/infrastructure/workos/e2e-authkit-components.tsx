"use client";

import type { ReactNode } from "react";

const E2E_ORGANIZATION_ID = "org_cutokyo_e2e";
const E2E_ACCESS_TOKEN = requiredE2EBearerToken();
const E2E_PERMISSIONS = [
  "audit:read",
  "devices:read",
  "devices:write",
  "groups:read",
  "groups:write",
  "identity:write",
  "members:write",
  "policies:read",
  "policies:write",
  "sessions:read",
  "telemetry:read",
  "widgets:users-table:manage",
] as const;
const E2E_USER = {
  createdAt: "2026-01-01T00:00:00.000Z",
  email: "admin@cutokyo.test",
  emailVerified: true,
  firstName: "E2E",
  id: "user_cutokyo_e2e_admin",
  lastName: "Administrator",
  lastSignInAt: "2026-01-02T00:00:00.000Z",
  name: "E2E Administrator",
  profilePictureUrl: null,
};

type AuthKitProviderProps = {
  children: ReactNode;
};

export function AuthKitProvider({ children }: AuthKitProviderProps) {
  return children;
}

export function useAuth() {
  return {
    loading: false,
    organizationId: E2E_ORGANIZATION_ID,
    permissions: [...E2E_PERMISSIONS],
    refreshAuth: async () => ({ user: E2E_USER }),
    role: "admin",
    roles: ["admin"],
    signOut: async () => undefined,
    switchToOrganization: async () => ({ organizationId: E2E_ORGANIZATION_ID }),
    user: E2E_USER,
  };
}

function requiredE2EBearerToken() {
  const token = process.env.NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN?.trim() ?? "";
  if (token.length < 32) {
    throw new Error("NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN is required for the E2E AuthKit adapter");
  }
  return token;
}

export function useAccessToken() {
  return {
    accessToken: E2E_ACCESS_TOKEN,
    error: undefined as Error | undefined,
    getAccessToken: async () => E2E_ACCESS_TOKEN,
    loading: false,
    refresh: async () => E2E_ACCESS_TOKEN,
  };
}

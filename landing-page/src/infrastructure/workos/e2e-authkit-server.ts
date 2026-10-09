import { NextResponse } from "next/server";

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
];
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

export function authkitProxy() {
  return () => NextResponse.next();
}

export function handleAuth({ returnPathname = "/admin" }: { returnPathname?: string } = {}) {
  return async (request: Request) => NextResponse.redirect(new URL(returnPathname, request.url));
}

export async function getSignInUrl({ returnTo = "/admin" }: { returnTo?: string } = {}) {
  return new URL(returnTo, requiredAppUrl()).toString();
}

export async function signOut() {
  return undefined;
}

export async function withAuth() {
  return {
    accessToken: E2E_ACCESS_TOKEN,
    organizationId: E2E_ORGANIZATION_ID,
    permissions: E2E_PERMISSIONS,
    role: "admin",
    roles: ["admin"],
    user: E2E_USER,
  };
}

export function getWorkOS() {
  return {
    adminPortal: {
      generateLink: async () => ({ link: `${requiredAppUrl()}/admin/enterprise?e2ePortal=1` }),
    },
    organizationDomains: {
      createOrganizationDomain: async ({ domain, organizationId }: Record<string, string>) => ({
        domain,
        id: "org_domain_cutokyo_e2e",
        organizationId,
      }),
    },
    organizations: {
      createOrganization: async ({ name }: { name: string }) => ({
        id: E2E_ORGANIZATION_ID,
        name,
      }),
      deleteOrganization: async () => undefined,
      getOrganization: async () => ({ id: E2E_ORGANIZATION_ID, name: "Cutokyo E2E" }),
    },
    userManagement: {
      createOrganizationMembership: async () => ({ id: "om_cutokyo_e2e" }),
      listInvitations: async () => ({
        data: [
          {
            email: "pending@cutokyo.test",
            expiresAt: "2030-01-08T00:00:00.000Z",
            id: "inv_cutokyo_e2e",
            roleSlug: "member",
            state: "pending",
          },
        ],
        listMetadata: {},
      }),
      listOrganizationMemberships: async () => ({
        data: [
          {
            role: { slug: "admin" },
            roles: [{ slug: "admin" }],
            userId: E2E_USER.id,
          },
          {
            role: { slug: "member" },
            roles: [{ slug: "member" }],
            userId: "user_cutokyo_e2e_member",
          },
        ],
        listMetadata: {},
      }),
      listUsers: async () => ({
        data: [
          E2E_USER,
          {
            email: "member@cutokyo.test",
            firstName: "E2E",
            id: "user_cutokyo_e2e_member",
            lastName: "Member",
            profilePictureUrl: null,
          },
        ],
        listMetadata: {},
      }),
      sendInvitation: async ({ email, roleSlug }: { email: string; roleSlug: string }) => ({
        email,
        expiresAt: "2030-01-08T00:00:00.000Z",
        id: "inv_cutokyo_e2e_created",
        roleSlug,
        state: "pending",
      }),
    },
    widgets: {
      createToken: async () => ({ token: "cutokyo-e2e-widget-token" }),
    },
  };
}

function requiredE2EBearerToken() {
  const token = process.env.NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN?.trim() ?? "";
  if (token.length < 32) {
    throw new Error("NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN is required for the E2E AuthKit adapter");
  }
  return token;
}

function requiredAppUrl() {
  const value = process.env.NEXT_PUBLIC_APP_URL?.trim();
  if (!value) throw new Error("NEXT_PUBLIC_APP_URL is required for the E2E AuthKit adapter");
  return value.replace(/\/$/, "");
}

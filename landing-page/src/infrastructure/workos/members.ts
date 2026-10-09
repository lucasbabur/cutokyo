import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { NextResponse } from "next/server";

import { JsonBodyTooLargeError, readBoundedJson } from "@/shared/http/bounded-json";

import { isWorkosConfigured } from "./config";

import type { NextRequest } from "next/server";

const PAGE_SIZE = 100;
const MAX_ORGANIZATION_MEMBERS = 5_000;
const MAX_INVITATION_REQUEST_BYTES = 8 * 1024;

class InvitationInputError extends Error {}

type OrganizationMember = {
  email: string;
  firstName: string | null;
  id: string;
  lastName: string | null;
  name: string;
  profilePictureUrl: string | null;
  roles: string[];
};

type OrganizationInvitation = {
  email: string;
  expiresAt: string;
  id: string;
  role: string | null;
  state: "accepted" | "expired" | "pending" | "revoked";
};

export async function listWorkosOrganizationMembers() {
  if (!isWorkosConfigured()) {
    return NextResponse.json(
      { message: "Organization identity is not configured." },
      { status: 503 },
    );
  }
  try {
    const auth = await withAuth();
    if (!auth.user) {
      return NextResponse.json({ message: "Authentication is required." }, { status: 401 });
    }
    if (!auth.organizationId) {
      return NextResponse.json({ message: "Select an organization first." }, { status: 400 });
    }
    const organizationId = auth.organizationId;
    if (!auth.permissions?.includes("telemetry:read")) {
      return NextResponse.json(
        { message: "The telemetry:read permission is required." },
        { status: 403 },
      );
    }
    const workos = getWorkOS();
    const [organization, users, memberships, invitations] = await Promise.all([
      workos.organizations.getOrganization(organizationId),
      collectPages((after) =>
        workos.userManagement.listUsers({
          after: after ?? null,
          limit: PAGE_SIZE,
          organizationId,
        }),
      ),
      collectPages((after) =>
        workos.userManagement.listOrganizationMemberships({
          after: after ?? null,
          limit: PAGE_SIZE,
          organizationId,
        }),
      ),
      collectPages((after) =>
        workos.userManagement.listInvitations({
          after: after ?? null,
          limit: PAGE_SIZE,
          organizationId,
        }),
      ),
    ]);
    const rolesByUser = new Map(
      memberships.map((membership) => [
        membership.userId,
        (membership.roles ?? [membership.role]).map((role) => role.slug),
      ]),
    );
    const members: OrganizationMember[] = users.map((user) => ({
      email: user.email,
      firstName: user.firstName,
      id: user.id,
      lastName: user.lastName,
      name:
        user.firstName || user.lastName
          ? [user.firstName, user.lastName].filter(Boolean).join(" ")
          : user.email,
      profilePictureUrl: safeProfilePictureUrl(user.profilePictureUrl),
      roles: rolesByUser.get(user.id) ?? [],
    }));
    const visibleInvitations: OrganizationInvitation[] = invitations.map((invitation) => ({
      email: invitation.email,
      expiresAt: invitation.expiresAt,
      id: invitation.id,
      role: invitation.roleSlug,
      state: invitation.state,
    }));
    return NextResponse.json({
      invitations: visibleInvitations,
      members,
      organization: { id: organization.id, name: organization.name },
    });
  } catch {
    return NextResponse.json({ message: "Organization directory lookup failed." }, { status: 502 });
  }
}

export async function inviteWorkosOrganizationMember(request: NextRequest) {
  if (!isWorkosConfigured()) {
    return NextResponse.json(
      { message: "Organization identity is not configured." },
      { status: 503 },
    );
  }
  try {
    const auth = await withAuth();
    if (!auth.user) {
      return NextResponse.json({ message: "Authentication is required." }, { status: 401 });
    }
    if (!auth.organizationId) {
      return NextResponse.json({ message: "Select an organization first." }, { status: 400 });
    }
    if (!canManageMembers(auth.permissions ?? [])) {
      return NextResponse.json(
        { message: "Your role cannot invite organization members." },
        { status: 403 },
      );
    }
    const input = await parseInvitationRequest(request);
    const invitation = await getWorkOS().userManagement.sendInvitation({
      email: input.email,
      expiresInDays: 7,
      inviterUserId: auth.user.id,
      organizationId: auth.organizationId,
      roleSlug: input.role,
    });
    return NextResponse.json(
      {
        invitation: {
          email: invitation.email,
          expiresAt: invitation.expiresAt,
          id: invitation.id,
          role: invitation.roleSlug,
          state: invitation.state,
        },
      },
      { status: 201 },
    );
  } catch (error) {
    if (error instanceof InvitationInputError) {
      return NextResponse.json({ message: error.message }, { status: 400 });
    }
    return NextResponse.json({ message: "Invitation could not be sent." }, { status: 502 });
  }
}

function canManageMembers(permissions: string[]) {
  return permissions.some((permission) =>
    ["groups:write", "members:write", "widgets:users-table:manage"].includes(permission),
  );
}

async function parseInvitationRequest(
  request: NextRequest,
): Promise<{ email: string; role: string }> {
  let body: unknown;
  try {
    body = await readBoundedJson<unknown>(
      request,
      "Invitation request",
      MAX_INVITATION_REQUEST_BYTES,
    );
  } catch (error) {
    if (error instanceof JsonBodyTooLargeError) {
      throw new InvitationInputError("Invitation request is too large.");
    }
    throw new InvitationInputError("Invitation request must contain valid JSON.");
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    throw new InvitationInputError("Invitation request must be a JSON object.");
  }
  const { email: rawEmail, role: rawRole } = body as { email?: unknown; role?: unknown };
  const email = typeof rawEmail === "string" ? rawEmail.trim().toLowerCase() : "";
  const role = typeof rawRole === "string" ? rawRole.trim().toLowerCase() : "member";
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email) || email.length > 254) {
    throw new InvitationInputError("Enter a valid employee email address.");
  }
  if (!/^[a-z0-9][a-z0-9_-]{0,47}$/.test(role)) {
    throw new InvitationInputError("Select a valid organization role.");
  }
  return { email, role };
}

function safeProfilePictureUrl(value: string | null): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" ? url.toString() : null;
  } catch {
    return null;
  }
}

async function collectPages<T>(
  load: (after: string | undefined) => Promise<{
    data: T[];
    listMetadata: { after?: string | null };
  }>,
): Promise<T[]> {
  const values: T[] = [];
  let after: string | undefined;
  do {
    const page = await load(after);
    values.push(...page.data);
    after = page.listMetadata.after ?? undefined;
    if (values.length > MAX_ORGANIZATION_MEMBERS) {
      throw new Error("Organization member limit exceeded");
    }
  } while (after);
  return values;
}

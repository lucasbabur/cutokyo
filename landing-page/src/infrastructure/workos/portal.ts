import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { NextResponse } from "next/server";

import { JsonBodyTooLargeError, readBoundedJson } from "@/shared/http/bounded-json";

import { isWorkosConfigured } from "./config";

import type { NextRequest } from "next/server";

const MAX_PORTAL_REQUEST_BYTES = 4 * 1024;
const ALLOWED_INTENTS = ["dsync", "sso"] as const;
type PortalIntent = (typeof ALLOWED_INTENTS)[number];

class PortalInputError extends Error {}

export async function createIdentityPortalLink(request: NextRequest) {
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
    if (!canManageIdentity(auth.permissions ?? [])) {
      return NextResponse.json(
        { message: "Your role cannot configure organization identity." },
        { status: 403 },
      );
    }
    const intent = await parseIntent(request);
    const returnUrl = new URL("/admin/enterprise", request.url).toString();
    const portal = await getWorkOS().adminPortal.generateLink({
      adminEmails: [auth.user.email],
      intent,
      organization: auth.organizationId,
      returnUrl,
      successUrl: returnUrl,
    });
    return NextResponse.json({ link: portal.link });
  } catch (error) {
    if (error instanceof PortalInputError) {
      return NextResponse.json({ message: error.message }, { status: 400 });
    }
    return NextResponse.json({ message: "Identity setup could not be opened." }, { status: 502 });
  }
}

function canManageIdentity(permissions: string[]) {
  return permissions.some((permission) =>
    ["groups:write", "identity:write", "widgets:users-table:manage"].includes(permission),
  );
}

async function parseIntent(request: NextRequest): Promise<PortalIntent> {
  let body: unknown;
  try {
    body = await readBoundedJson<unknown>(
      request,
      "Identity setup request",
      MAX_PORTAL_REQUEST_BYTES,
    );
  } catch (error) {
    if (error instanceof JsonBodyTooLargeError) {
      throw new PortalInputError("Identity setup request is too large.");
    }
    throw new PortalInputError("Identity setup request must contain valid JSON.");
  }
  const intent =
    body && typeof body === "object" && !Array.isArray(body)
      ? (body as { intent?: unknown }).intent
      : undefined;
  if (typeof intent !== "string" || !ALLOWED_INTENTS.includes(intent as PortalIntent)) {
    throw new PortalInputError("Select SSO or directory sync setup.");
  }
  return intent as PortalIntent;
}

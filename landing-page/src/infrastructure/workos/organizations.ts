import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { NextResponse } from "next/server";

import { JsonBodyTooLargeError, readBoundedJson } from "@/shared/http/bounded-json";

import { isWorkosConfigured } from "./config";

import type { NextRequest } from "next/server";

type CreateOrganizationBody = {
  domain?: unknown;
  name?: unknown;
};

class OrganizationInputError extends Error {}

const MAX_ORGANIZATION_REQUEST_BYTES = 16 * 1024;

export async function createWorkosOrganization(request: NextRequest) {
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
    const input = await parseOrganizationRequest(request);
    const workos = getWorkOS();
    const organization = await workos.organizations.createOrganization({
      metadata: { created_by: auth.user.id, product: "cutokyo" },
      name: input.name,
    });
    try {
      await workos.userManagement.createOrganizationMembership({
        organizationId: organization.id,
        roleSlug: "admin",
        userId: auth.user.id,
      });
      const domain = input.domain
        ? await workos.organizationDomains.createOrganizationDomain({
            domain: input.domain,
            organizationId: organization.id,
          })
        : null;
      return NextResponse.json(
        { domain, organization: { id: organization.id, name: organization.name } },
        { status: 201 },
      );
    } catch (error) {
      await workos.organizations.deleteOrganization(organization.id).catch(() => undefined);
      throw error;
    }
  } catch (error) {
    if (error instanceof OrganizationInputError) {
      return NextResponse.json({ message: error.message }, { status: 400 });
    }
    return NextResponse.json({ message: "Organization creation failed." }, { status: 502 });
  }
}

async function parseOrganizationRequest(
  request: NextRequest,
): Promise<{ domain: string; name: string }> {
  let body: unknown;
  try {
    body = await readBoundedJson<unknown>(
      request,
      "Organization request",
      MAX_ORGANIZATION_REQUEST_BYTES,
    );
  } catch (error) {
    if (error instanceof JsonBodyTooLargeError) {
      throw new OrganizationInputError(
        `Request body must not exceed ${MAX_ORGANIZATION_REQUEST_BYTES} bytes.`,
      );
    }
    throw new OrganizationInputError("Request body must contain valid JSON.");
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    throw new OrganizationInputError("Request body must be a JSON object.");
  }
  return parseOrganizationInput(body as CreateOrganizationBody);
}

function parseOrganizationInput(body: CreateOrganizationBody): { domain: string; name: string } {
  const name = typeof body.name === "string" ? body.name.trim() : "";
  const domain = typeof body.domain === "string" ? body.domain.trim().toLowerCase() : "";
  if (name.length < 2 || name.length > 80) {
    throw new OrganizationInputError("Organization name must contain 2 to 80 characters.");
  }
  if (domain && !/^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/.test(domain)) {
    throw new OrganizationInputError("Enter a valid organization domain.");
  }
  return { domain, name };
}

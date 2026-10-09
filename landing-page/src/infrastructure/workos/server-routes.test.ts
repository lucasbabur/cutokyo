import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { NextRequest } from "next/server";
import { beforeEach, vi } from "vitest";

import { listWorkosOrganizationMembers } from "./members";
import { createWorkosOrganization } from "./organizations";
import { createWorkosWidgetToken } from "./widget-token";

vi.mock("@workos-inc/authkit-nextjs", () => ({
  getWorkOS: vi.fn(),
  withAuth: vi.fn(),
}));

vi.mock("./config", () => ({ isWorkosConfigured: () => true }));

describe("WorkOS server routes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(withAuth).mockResolvedValue({
      organizationId: "org-cutokyo",
      permissions: ["widgets:users-table:manage"],
      user: { id: "user-admin" },
    } as never);
  });

  it("keeps organization validation errors actionable", async () => {
    const request = new NextRequest("http://localhost/api/workos/organizations", {
      body: JSON.stringify({ domain: "invalid", name: "x" }),
      headers: { "content-type": "application/json" },
      method: "POST",
    });

    const response = await createWorkosOrganization(request);

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({
      message: "Organization name must contain 2 to 80 characters.",
    });
    expect(getWorkOS).not.toHaveBeenCalled();
  });

  it.each([
    ["invalid JSON", "not-json", "Request body must contain valid JSON."],
    ["a non-object body", "null", "Request body must be a JSON object."],
  ])("rejects %s before calling WorkOS", async (_case, body, message) => {
    const request = new NextRequest("http://localhost/api/workos/organizations", {
      body,
      headers: { "content-type": "application/json" },
      method: "POST",
    });

    const response = await createWorkosOrganization(request);

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({ message });
    expect(getWorkOS).not.toHaveBeenCalled();
  });

  it("rejects an oversized organization request before parsing", async () => {
    const request = new NextRequest("http://localhost/api/workos/organizations", {
      body: JSON.stringify({ name: "Cutokyo", padding: "x".repeat(16 * 1024) }),
      headers: { "content-type": "application/json" },
      method: "POST",
    });

    const response = await createWorkosOrganization(request);

    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toEqual({
      message: "Request body must not exceed 16384 bytes.",
    });
    expect(getWorkOS).not.toHaveBeenCalled();
  });

  it("does not expose WorkOS organization failure details", async () => {
    vi.mocked(getWorkOS).mockReturnValue({
      organizations: {
        createOrganization: vi.fn().mockRejectedValue(new Error("secret provider request id")),
      },
    } as never);
    const request = new NextRequest("http://localhost/api/workos/organizations", {
      body: JSON.stringify({ domain: "example.test", name: "Cutokyo" }),
      headers: { "content-type": "application/json" },
      method: "POST",
    });

    const response = await createWorkosOrganization(request);

    expect(response.status).toBe(502);
    await expect(response.json()).resolves.toEqual({ message: "Organization creation failed." });
  });

  it("does not expose WorkOS widget token failure details", async () => {
    vi.mocked(getWorkOS).mockReturnValue({
      widgets: {
        createToken: vi.fn().mockRejectedValue(new Error("secret widget diagnostic")),
      },
    } as never);
    const request = new NextRequest("http://localhost/api/workos/widgets/token?scope=users");

    const response = await createWorkosWidgetToken(request);

    expect(response.status).toBe(502);
    await expect(response.json()).resolves.toEqual({ message: "Widget token creation failed." });
  });

  it("lists active organization members with their WorkOS roles", async () => {
    vi.mocked(withAuth).mockResolvedValue({
      organizationId: "org-cutokyo",
      permissions: ["telemetry:read"],
      user: { id: "user-admin" },
    } as never);
    vi.mocked(getWorkOS).mockReturnValue({
      organizations: {
        getOrganization: vi.fn().mockResolvedValue({ id: "org-cutokyo", name: "Cutokyo Labs" }),
      },
      userManagement: {
        listInvitations: vi.fn().mockResolvedValue({ data: [], listMetadata: {} }),
        listOrganizationMemberships: vi.fn().mockResolvedValue({
          data: [
            {
              role: { slug: "member" },
              roles: [{ slug: "member" }, { slug: "developer" }],
              userId: "user-ada",
            },
          ],
          listMetadata: {},
        }),
        listUsers: vi.fn().mockResolvedValue({
          data: [
            {
              email: "ada@example.com",
              firstName: "Ada",
              id: "user-ada",
              lastName: "Lovelace",
              profilePictureUrl: "https://images.example.com/ada.png",
            },
          ],
          listMetadata: {},
        }),
      },
    } as never);

    const response = await listWorkosOrganizationMembers();

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({
      invitations: [],
      members: [
        {
          email: "ada@example.com",
          firstName: "Ada",
          id: "user-ada",
          lastName: "Lovelace",
          name: "Ada Lovelace",
          profilePictureUrl: "https://images.example.com/ada.png",
          roles: ["member", "developer"],
        },
      ],
      organization: { id: "org-cutokyo", name: "Cutokyo Labs" },
    });
  });

  it("requires telemetry permission before reading the organization directory", async () => {
    const response = await listWorkosOrganizationMembers();

    expect(response.status).toBe(403);
    expect(getWorkOS).not.toHaveBeenCalled();
  });
});

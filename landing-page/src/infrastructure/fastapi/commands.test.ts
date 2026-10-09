import { afterEach, describe, expect, it, vi } from "vitest";

import { env } from "@/shared/config/env";
import { MAX_JSON_RESPONSE_BYTES } from "@/shared/http/bounded-json";

import {
  analyzeCutokyoContextCommand,
  createWorkosOrganizationCommand,
  loadOrganizationAdminCommand,
  loadOrganizationSessionCommand,
  loadWorkosWidgetTokenCommand,
  saveOrganizationGroupCommand,
} from "./commands";

import type { OrganizationTelemetryCommandResult } from "./commands";

const emptyTelemetry: OrganizationTelemetryCommandResult = {
  byDevice: [],
  byGroup: [],
  byHarness: [],
  byModel: [],
  byProvider: [],
  byUser: [],
  cacheReadTokens: 0,
  context: {
    conversationHistory: 0,
    currentUserInput: 0,
    instructions: 0,
    mcpDefinitions: 0,
    mcpResults: 0,
    media: 0,
    otherContext: 0,
    retrievedDocuments: 0,
    toolDefinitions: 0,
    toolResults: 0,
  },
  errorEvents: 0,
  estimatedTokensSaved: 0,
  recentEvents: [],
  totalCostNanosUsd: 0,
  totalEvents: 0,
  totalRedactions: 0,
  totalTokens: 0,
  timeline: [],
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("FastAPI commands", () => {
  it("loads an encoded WorkOS widget scope", async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ token: "widget-token" }));
    vi.stubGlobal("fetch", fetch);

    await expect(loadWorkosWidgetTokenCommand("widgets:sso:manage all")).resolves.toBe(
      "widget-token",
    );
    expect(fetch).toHaveBeenCalledWith(
      "/api/workos/widgets/token?scope=widgets%3Asso%3Amanage%20all",
      { headers: { accept: "application/json" } },
    );
  });

  it("surfaces WorkOS command errors and validates organization responses", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ message: "Widget access denied" }, { status: 403 }))
      .mockResolvedValueOnce(Response.json({ organization: { id: 42 } }));
    vi.stubGlobal("fetch", fetch);

    await expect(loadWorkosWidgetTokenCommand("widgets:sso:manage")).rejects.toThrow(
      "Widget access denied",
    );
    await expect(
      createWorkosOrganizationCommand({ domain: "example.test", name: "Cutokyo" }),
    ).rejects.toThrow("Organization creation failed");
    expect(fetch.mock.calls[1]).toEqual([
      "/api/workos/organizations",
      {
        body: JSON.stringify({ domain: "example.test", name: "Cutokyo" }),
        headers: { "content-type": "application/json" },
        method: "POST",
      },
    ]);
  });

  it("uses stable WorkOS errors for non-JSON responses", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(new Response("gateway failure", { status: 502 }))
      .mockResolvedValueOnce(new Response("gateway failure", { status: 502 }));
    vi.stubGlobal("fetch", fetch);

    await expect(loadWorkosWidgetTokenCommand("users")).rejects.toThrow(
      "Identity authorization failed.",
    );
    await expect(
      createWorkosOrganizationCommand({ domain: "example.test", name: "Cutokyo" }),
    ).rejects.toThrow("Organization creation failed.");
  });

  it("forwards context analysis with cancellation and stable HTTP errors", async () => {
    const abortController = new AbortController();
    const result = { analysis: { categories: [] } };
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(Response.json(result))
      .mockResolvedValueOnce(Response.json({}, { status: 503 }));
    vi.stubGlobal("fetch", fetch);

    await expect(
      analyzeCutokyoContextCommand(
        {
          actor: { id: "user-123" },
          contextWindowTokens: 128_000,
          messages: [],
          resources: [],
          target: { model: "context-analysis", provider: "cutokyo" },
        },
        abortController.signal,
      ),
    ).resolves.toEqual(result);
    expect(fetch.mock.calls[0]?.[1]).toMatchObject({
      method: "POST",
      signal: abortController.signal,
    });
    await expect(
      analyzeCutokyoContextCommand({
        actor: { id: "user-123" },
        contextWindowTokens: 128_000,
        messages: [],
        resources: [],
        target: { model: "context-analysis", provider: "cutokyo" },
      }),
    ).rejects.toThrow("Cutokyo analysis failed with 503");
  });

  it("rejects a FastAPI response with an oversized declared body", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response("{}", {
          headers: { "content-length": String(MAX_JSON_RESPONSE_BYTES + 1) },
        }),
      ),
    );

    await expect(
      analyzeCutokyoContextCommand({
        actor: { id: "user-123" },
        contextWindowTokens: 128_000,
        messages: [],
        resources: [],
        target: { model: "context-analysis", provider: "cutokyo" },
      }),
    ).rejects.toThrow(`FastAPI response exceeded ${MAX_JSON_RESPONSE_BYTES} bytes.`);
  });

  it("loads only organization resources granted by permissions", async () => {
    const license = {
      detail: "Licensing is disabled.",
      features: [],
      limits: {},
      mode: "disabled",
      readOnly: false,
      state: "disabled",
      wouldBlock: false,
    };
    const fetch = vi
      .fn()
      .mockImplementation((url: string) =>
        Promise.resolve(Response.json(url.endsWith("/license/status") ? license : emptyTelemetry)),
      );
    vi.stubGlobal("fetch", fetch);
    const signal = new AbortController().signal;

    const result = await loadOrganizationAdminCommand(
      "org/with space",
      "access-token",
      signal,
      [],
      { groupId: "group/a", userId: "user+one" },
    );

    expect(result).toEqual({
      ...emptyTelemetry,
      auditEvents: [],
      devices: [],
      groups: [],
      invitations: [],
      license,
      members: [],
      organization: { id: "org/with space", name: "Organization" },
      plugins: [],
      policy: null,
      sessions: [],
    });
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(fetch.mock.calls[0]?.[0]).toBe(
      `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/org%2Fwith%20space/telemetry/summary?groupId=group%2Fa&userId=user%2Bone&limit=500`,
    );
    expect(fetch.mock.calls[0]?.[1]).toEqual({
      headers: { Authorization: "Bearer access-token" },
      signal,
    });
  });

  it("maps authorized mutation error details", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(Response.json({ detail: "Group membership is invalid" }, { status: 409 }));
    vi.stubGlobal("fetch", fetch);

    await expect(
      saveOrganizationGroupCommand("org-123", "access-token", {
        id: "group/123",
        memberIds: ["user-123"],
        name: "Platform",
      }),
    ).rejects.toThrow("Group membership is invalid");
    const [url, init] = fetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(
      `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/org-123/groups/group%2F123`,
    );
    expect(init.method).toBe("PUT");
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer access-token");
  });

  it("loads a specific encoded organization session", async () => {
    const detail = { id: "session/one", messages: [] };
    const fetch = vi.fn().mockResolvedValue(Response.json(detail));
    vi.stubGlobal("fetch", fetch);
    const signal = new AbortController().signal;

    await expect(
      loadOrganizationSessionCommand("org/one", "session/one", "access-token", signal),
    ).resolves.toEqual(detail);
    const [url, init] = fetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(
      `${env.NEXT_PUBLIC_FASTAPI_BASE_URL}/cutokyo/v1/organizations/org%2Fone/sessions/session%2Fone`,
    );
    expect(init.signal).toBe(signal);
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer access-token");
  });
});

import { describe, expect, it } from "vitest";

import type { SessionFilters } from "../contracts.js";
import { DEFAULT_SESSION_FILTERS } from "../domain/sessionSearch.js";
import { createBrowserFixtureClient } from "./browserAdapter.js";

const ALL: SessionFilters = DEFAULT_SESSION_FILTERS;

const JEV_CASES = [
  "onboarding-empty-history",
  "search-detail-resume",
  "retention-delete-confirmation",
  "proxy-capture-consent",
  "degraded-health-recovery",
  "mcp-plugin-inventory",
  "visual-keyboard-consistency",
] as const;

describe("isolated browser fixtures", () => {
  it.each(JEV_CASES)(
    "selects the validated %s case deterministically",
    async (caseName) => {
      const first = createBrowserFixtureClient(caseName);
      const second = createBrowserFixtureClient(caseName);
      expect(await first.getBootstrap()).toEqual(await second.getBootstrap());
      expect(await first.searchSessions(ALL)).toEqual(
        await second.searchSessions(ALL),
      );
    },
  );

  it("rejects an unvalidated selector only inside explicit fixture mode", () => {
    expect(() => createBrowserFixtureClient("operator-data")).toThrow(
      "Unknown isolated browser fixture",
    );
  });

  it("starts onboarding and empty history without synthetic dashboard totals", async () => {
    const client = createBrowserFixtureClient("onboarding-empty");
    expect((await client.getBootstrap()).onboardingComplete).toBe(false);
    expect((await client.getDashboard()).sessions).toEqual([]);
    expect((await client.searchSessions(ALL)).sessions).toEqual([]);
  });

  it("searches and resumes the exact native identity", async () => {
    const client = createBrowserFixtureClient("search-resume");
    const response = await client.searchSessions({
      ...ALL,
      text: "JEV exact resume needle 73A9",
    });
    expect(response.sessions).toHaveLength(1);
    expect(response.sessions[0]?.nativeResumeId).toBe("claude-jev-73A9");
    await client.resumeSession(response.sessions[0]!.id);
    expect(client.fixtureAudit().resumeRequests).toEqual(["claude-jev-73A9"]);
  });

  it("deletes only the previewed session and preserves its neighbor", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    const preview = await client.previewSessionDeletion("delete-me-73A9");
    await client.deleteSession("delete-me-73A9", preview.previewToken);
    expect(client.fixtureAudit().sessionIds).toEqual(["keep-me-73A9"]);
  });

  it("applies exactly the immutable retention preview", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    const preview = await client.previewRetention(30);
    expect(preview.sessionIds).toEqual(["delete-me-73A9"]);
    const receipt = await client.applyRetention(preview.previewToken);
    expect(receipt.sessions).toBe(preview.sessionIds.length);
    expect(client.fixtureAudit().sessionIds).toEqual(["keep-me-73A9"]);
  });

  it("does not mutate history when delete-all confirmation is rejected", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    const before = client.fixtureAudit().sessionIds;
    await expect(client.deleteAll("cancel")).rejects.toThrow(
      "DELETE ALL LOCAL HISTORY",
    );
    expect(client.fixtureAudit().sessionIds).toEqual(before);
  });

  it("edits one native MCP source without changing other installations", async () => {
    const client = createBrowserFixtureClient("mcp-plugin-inventory");
    const before = await client.getInventory();
    const target = before.items.find((item) => item.name === "docs-73A9");
    expect(target).toBeDefined();
    const source = await client.getInventoryDocument(target!.id);
    const content = JSON.stringify({ command: "local-docs", args: [] });
    await client.saveInventoryDocument(target!.id, source.revision, content);
    expect((await client.getInventoryDocument(target!.id)).content).toBe(
      content,
    );
    const after = await client.getInventory();
    expect(after.items).toEqual(before.items);
    await expect(
      client.saveInventoryDocument(target!.id, source.revision, content),
    ).rejects.toThrow("changed");
  });

  it("reports actual proxy inactivity before and after an unavailable activation", async () => {
    const client = createBrowserFixtureClient("proxy-capture-consent");
    const preview = await client.previewProxy();
    const status = await client.getProxyStatus();
    for (const disclosure of [
      preview.fallbackBehavior,
      status.detail,
      ...status.meta.notices,
    ]) {
      expect(disclosure).toContain(
        "Native capture configuration is unaffected",
      );
      expect(disclosure).toContain(
        "live capture remains unknown until evidence arrives",
      );
      expect(disclosure).not.toMatch(
        /Native capture remains active|active where (configured|installed)/i,
      );
    }
    expect(client.fixtureAudit().proxyActive).toBe(false);
    await expect(
      client.setProxyEnabled(true, preview.consentToken),
    ).rejects.toThrow("unavailable");
    expect(client.fixtureAudit().proxyActive).toBe(
      (await client.getProxyStatus()).proxyStatus === "active",
    );
    expect(client.fixtureAudit().proxyActive).toBe(false);
  });

  it("persists only appearance through the desktop preference patch", async () => {
    const client = createBrowserFixtureClient("visual-keyboard");
    const before = await client.getSettings();
    expect(before.appearance).toBe("system");
    const saved = await client.patchDesktopPreferences({ appearance: "dark" });
    expect(saved).toEqual({ ...before, appearance: "dark" });
    expect(await client.getSettings()).toEqual(saved);
    expect(
      (await createBrowserFixtureClient("visual-keyboard").getSettings())
        .appearance,
    ).toBe("system");
  });

  it("recovers writer lock independently of surviving quarantine", async () => {
    const client = createBrowserFixtureClient("health-degraded");
    const next = await client.retryHealth("writer_lock");
    expect(
      next.dimensions.find((item) => item.id === "writer_lock")?.state,
    ).toBe("healthy");
    expect(
      next.dimensions.find((item) => item.id === "quarantine")?.state,
    ).toBe("degraded");
    expect(next.currentQuarantineCount).toBe(1);
    expect(next.firstAffectedObservationId).toBe("spool:malformed-73A9");
  });
});

it("removes guard APIs/settings while retaining explicit proxy consent", async () => {
  const client = createBrowserFixtureClient("proxy-capture-consent");
  expect(client).not.toHaveProperty("getGuards");
  expect(client).not.toHaveProperty("setOutgoingGuardEnabled");
  expect(await client.getSettings()).not.toHaveProperty(
    "outgoing_guard_enabled",
  );
  await expect(client.setProxyEnabled(true, null)).rejects.toThrow(
    "explicit consent preview",
  );
  const preview = await client.previewProxy();
  expect(preview.redactionBehavior).toContain(
    "Provider requests pass unchanged",
  );
  await expect(
    client.setProxyEnabled(true, preview.consentToken),
  ).rejects.toThrow("no provider-proxy listener");
  expect((await client.getProxyStatus()).proxyStatus).toBe("unavailable");
  const stopped = await client.setProxyEnabled(false, null);
  expect(stopped.proxyStatus).toBe("unavailable");
  expect(stopped.proxyEnabled).toBe(false);
  expect(stopped.contextBreakdownAvailable).toBe(false);
});

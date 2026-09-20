import { describe, expect, it } from "vitest";

import type { SessionFilters } from "../contracts.js";
import { createBrowserFixtureClient } from "./browserAdapter.js";

const ALL: SessionFilters = {
  text: "",
  harness: "all",
  project: "",
  branch: "",
  dateRange: "all",
  tool: "",
  skill: "",
  agent: "",
};

const JEV_CASES = [
  "onboarding-empty-history",
  "search-detail-resume",
  "retention-delete-confirmation",
  "guard-proxy-coverage-language",
  "analysis-preview-cancel",
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

  it("keeps MCP broker items while toggling one route", async () => {
    const client = createBrowserFixtureClient("mcp-plugin-inventory");
    const before = await client.getInventory();
    const target = before.items.find((item) => item.name === "docs-73A9");
    expect(target).toBeDefined();
    const after = await client.setMcpEnabled(target!.id, false);
    expect(after.items).toHaveLength(before.items.length);
    expect(after.items.find((item) => item.id === target!.id)?.state).toBe(
      "disabled",
    );
    expect(
      after.items.find((item) => item.id === "mcp-cutokyo-search")?.state,
    ).toBe("enabled");
  });

  it("records no outbound request after an analysis is cancelled", async () => {
    const client = createBrowserFixtureClient("analysis-cancel");
    const preview = await client.previewAnalysis(["analysis-73A9"]);
    await client.cancelAnalysis(preview.requestId);
    await expect(client.runAnalysis(preview.previewToken)).rejects.toThrow(
      "cancelled",
    );
    expect(client.fixtureAudit().outboundAnalysisRequests).toBe(0);
  });

  it("retries analysis idempotently after a bounded error", async () => {
    const client = createBrowserFixtureClient("analysis-error");
    const preview = await client.previewAnalysis(["analysis-73A9"]);
    await expect(client.runAnalysis(preview.previewToken)).rejects.toThrow(
      "timed out",
    );
    const first = await client.runAnalysis(preview.previewToken);
    const second = await client.runAnalysis(preview.previewToken);
    expect(first).toEqual(second);
    expect(client.fixtureAudit().outboundAnalysisRequests).toBe(2);
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

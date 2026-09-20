import { describe, expect, it } from "vitest";

import type { SessionRecord, UsageRecord } from "../contracts.js";
import { dashboardSessions } from "../fixtures/scenarios.js";
import {
  estimateCost,
  quotaPercent,
  reconcileUsage,
  usageByHarness,
} from "./reconcile.js";

function usageWith(
  overrides: Partial<UsageRecord>,
  sourceTier: number,
): UsageRecord {
  const base = dashboardSessions[0]?.usage[0];
  if (base === undefined) throw new Error("Fixture usage is missing.");
  return {
    ...base,
    ...overrides,
    provenance: { ...base.provenance, sourceTier },
  };
}

function sessionWithUsage(...usage: UsageRecord[]): SessionRecord {
  const base = dashboardSessions[0];
  if (base === undefined) throw new Error("Fixture session is missing.");
  return { ...base, usage };
}

describe("usage reconciliation", () => {
  it("reconciles fixture totals used by the chart and raw table", () => {
    const totals = reconcileUsage(dashboardSessions);
    expect(totals.inputTokens).toMatchObject({
      value: 184_200,
      unknownRecords: 0,
    });
    expect(totals.outputTokens).toMatchObject({
      value: 42_810,
      unknownRecords: 0,
    });
    expect(
      usageByHarness(dashboardSessions).map((row) => row.totalTokens),
    ).toEqual([81_000, 95_510, 50_500]);
  });

  it("counts the best source once and retains its duplicate key", () => {
    const lowerPriority = usageWith(
      { nativeUsageKey: "native-request", inputTokens: 900 },
      8,
    );
    const winner = usageWith(
      { nativeUsageKey: "native-request", inputTokens: 100 },
      3,
    );
    const totals = reconcileUsage([
      sessionWithUsage(lowerPriority),
      { ...sessionWithUsage(winner), id: "second-session" },
    ]);
    expect(totals.inputTokens.value).toBe(100);
    expect(totals.records).toEqual([winner]);
    expect(totals.duplicateKeys).toEqual(["native-request"]);
  });

  it("preserves unknown metrics instead of manufacturing zero", () => {
    const unknown = usageWith(
      { inputTokens: null, outputTokens: null, cacheReadTokens: null },
      1,
    );
    const totals = reconcileUsage([sessionWithUsage(unknown)]);
    expect(totals.inputTokens).toEqual({
      value: null,
      knownRecords: 0,
      unknownRecords: 1,
    });
    expect(
      usageByHarness([sessionWithUsage(unknown)])[0]?.totalTokens,
    ).toBeNull();
  });

  it("uses half-open price validity and labels a computed price as estimated", () => {
    const base = dashboardSessions[0];
    if (base === undefined) throw new Error("Fixture session is missing.");
    const atBoundary = { ...base, startedAt: "2026-10-01T00:00:00Z" };
    const price = {
      provider: "fixture",
      model: base.usage[0]?.model ?? "",
      currency: "USD",
      inputMicrosPerMillion: 1_000_000,
      outputMicrosPerMillion: 1_000_000,
      validFrom: "2026-09-01T00:00:00Z",
      validUntil: "2026-10-01T00:00:00Z",
      confidence: "estimated" as const,
      label: "Fixture estimate",
      provenance: base.provenance,
    };
    expect(estimateCost([atBoundary], [price])).toMatchObject({
      micros: null,
      label: "unavailable",
    });
    expect(estimateCost([base], [price])).toMatchObject({
      micros: 81_000,
      label: "estimated",
    });
  });

  it("renders a quota meter only when limit and remaining are both established", () => {
    const base = {
      name: "fixture",
      used: null,
      resetsAt: null,
      confidence: "unknown" as const,
      label: "unknown",
      provenance: dashboardSessions[0]!.provenance,
    };
    expect(quotaPercent({ ...base, limit: null, remaining: null })).toBeNull();
    expect(quotaPercent({ ...base, limit: 200, remaining: 72 })).toBe(36);
  });
});

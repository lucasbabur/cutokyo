import { describe, expect, it } from "vitest";

import type { SessionRecord, UsageRecord } from "../contracts.js";
import { dashboardSessions } from "../fixtures/scenarios.js";
import {
  costCaption,
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

  it("counts unpriced records instead of presenting a partial sum as complete", () => {
    const base = dashboardSessions[0];
    if (base === undefined) throw new Error("Fixture session is missing.");
    const price = {
      provider: "fixture",
      model: base.usage[0]?.model ?? "",
      currency: "USD",
      inputMicrosPerMillion: 1_000_000,
      outputMicrosPerMillion: 1_000_000,
      validFrom: "2026-09-01T00:00:00Z",
      validUntil: null,
      confidence: "estimated" as const,
      label: "Fixture estimate",
      provenance: base.provenance,
    };
    const cost = estimateCost(dashboardSessions, [price]);
    expect(cost).toEqual({
      micros: 81_000,
      pricedRecords: 1,
      unpricedRecords: 2,
      label: "estimated",
    });
    expect(costCaption(cost)).toBe(
      "Partial · 1 of 3 usage records priced · Estimated",
    );
  });

  it("captions every cost source and the unavailable reason", () => {
    const reported = usageWith(
      { nativeUsageKey: "reported", providerCostMicros: 1_250_000 },
      1,
    );
    const unpricedModel = usageWith(
      { nativeUsageKey: "unpriced", model: null, providerCostMicros: null },
      1,
    );
    const base = dashboardSessions[0];
    if (base === undefined) throw new Error("Fixture session is missing.");
    const price = {
      provider: "fixture",
      model: base.usage[0]?.model ?? "",
      currency: "USD",
      inputMicrosPerMillion: 1_000_000,
      outputMicrosPerMillion: 1_000_000,
      validFrom: "2026-09-01T00:00:00Z",
      validUntil: null,
      confidence: "estimated" as const,
      label: "Fixture estimate",
      provenance: base.provenance,
    };
    const estimable = usageWith({ nativeUsageKey: "estimable" }, 1);

    const providerOnly = estimateCost([sessionWithUsage(reported)], []);
    expect(providerOnly).toMatchObject({
      micros: 1_250_000,
      label: "provider_reported",
    });
    expect(costCaption(providerOnly)).toBe("Reported by harness");

    const estimatedOnly = estimateCost([sessionWithUsage(estimable)], [price]);
    expect(costCaption(estimatedOnly)).toBe("Estimated");

    const mixed = estimateCost(
      [sessionWithUsage(reported, estimable)],
      [price],
    );
    expect(mixed).toMatchObject({ micros: 1_331_000, label: "mixed" });
    expect(costCaption(mixed)).toBe("Reported by harness + estimated");

    const partialReported = estimateCost(
      [sessionWithUsage(reported, unpricedModel)],
      [],
    );
    expect(costCaption(partialReported)).toBe(
      "Partial · 1 of 2 usage records priced · Reported by harness",
    );

    const unavailable = estimateCost([sessionWithUsage(unpricedModel)], []);
    expect(unavailable).toEqual({
      micros: null,
      pricedRecords: 0,
      unpricedRecords: 1,
      label: "unavailable",
    });
    expect(costCaption(unavailable)).toBe(
      "No cost reported: subscription or unpriced models",
    );
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

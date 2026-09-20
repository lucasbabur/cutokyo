import type {
  Harness,
  PriceFact,
  QuotaFact,
  SessionRecord,
  UsageRecord,
} from "../contracts.js";

interface OptionalTotal {
  readonly value: number | null;
  readonly knownRecords: number;
  readonly unknownRecords: number;
}

export interface UsageReconciliation {
  readonly inputTokens: OptionalTotal;
  readonly outputTokens: OptionalTotal;
  readonly cacheReadTokens: OptionalTotal;
  readonly cacheWriteTokens: OptionalTotal;
  readonly providerCostMicros: OptionalTotal;
  readonly records: readonly UsageRecord[];
  readonly duplicateKeys: readonly string[];
  readonly sessions: number;
}

export interface HarnessUsage {
  readonly harness: Harness;
  readonly sessionCount: number;
  readonly inputTokens: number | null;
  readonly outputTokens: number | null;
  readonly totalTokens: number | null;
}

function totalOptional(
  records: readonly UsageRecord[],
  select: (record: UsageRecord) => number | null,
): OptionalTotal {
  const values = records.map(select);
  const known = values.filter((value): value is number => value !== null);
  return {
    value:
      known.length === 0
        ? null
        : known.reduce((total, value) => total + value, 0),
    knownRecords: known.length,
    unknownRecords: values.length - known.length,
  };
}

/**
 * Reconciles by native usage identity. A lower source-tier number wins; conflicting
 * observations remain countable evidence but never enter totals twice.
 */
export function reconcileUsage(
  sessions: readonly SessionRecord[],
): UsageReconciliation {
  const winners = new Map<string, UsageRecord>();
  const duplicates = new Set<string>();
  for (const session of sessions) {
    for (const record of session.usage) {
      const current = winners.get(record.nativeUsageKey);
      if (current === undefined) {
        winners.set(record.nativeUsageKey, record);
      } else {
        duplicates.add(record.nativeUsageKey);
        if (record.provenance.sourceTier < current.provenance.sourceTier) {
          winners.set(record.nativeUsageKey, record);
        }
      }
    }
  }
  const records = [...winners.values()];
  return {
    inputTokens: totalOptional(records, (record) => record.inputTokens),
    outputTokens: totalOptional(records, (record) => record.outputTokens),
    cacheReadTokens: totalOptional(records, (record) => record.cacheReadTokens),
    cacheWriteTokens: totalOptional(
      records,
      (record) => record.cacheWriteTokens,
    ),
    providerCostMicros: totalOptional(
      records,
      (record) => record.providerCostMicros,
    ),
    records,
    duplicateKeys: [...duplicates].sort(),
    sessions: sessions.length,
  };
}

export function usageByHarness(
  sessions: readonly SessionRecord[],
): readonly HarnessUsage[] {
  const order: readonly Harness[] = ["claude_code", "codex", "opencode"];
  return order.map((harness) => {
    const harnessSessions = sessions.filter(
      (session) => session.harness === harness,
    );
    const totals = reconcileUsage(harnessSessions);
    const input = totals.inputTokens.value;
    const output = totals.outputTokens.value;
    return {
      harness,
      sessionCount: harnessSessions.length,
      inputTokens: input,
      outputTokens: output,
      totalTokens: input === null || output === null ? null : input + output,
    };
  });
}

export interface EstimatedCost {
  readonly micros: number | null;
  readonly pricedRecords: number;
  readonly unpricedRecords: number;
  readonly label: "estimated" | "provider_reported" | "unavailable";
}

export function estimateCost(
  sessions: readonly SessionRecord[],
  prices: readonly PriceFact[],
): EstimatedCost {
  const usage = reconcileUsage(sessions).records;
  let micros = 0;
  let pricedRecords = 0;
  let providerReported = 0;
  for (const record of usage) {
    if (record.providerCostMicros !== null) {
      micros += record.providerCostMicros;
      pricedRecords += 1;
      providerReported += 1;
      continue;
    }
    const session = sessions.find((candidate) =>
      candidate.usage.includes(record),
    );
    const occurredAt = session?.startedAt;
    const price = prices.find(
      (candidate) =>
        candidate.model === record.model &&
        occurredAt !== undefined &&
        candidate.validFrom <= occurredAt &&
        (candidate.validUntil === null || occurredAt < candidate.validUntil),
    );
    if (price === undefined) continue;
    let established = false;
    if (record.inputTokens !== null && price.inputMicrosPerMillion !== null) {
      micros += (record.inputTokens * price.inputMicrosPerMillion) / 1_000_000;
      established = true;
    }
    if (record.outputTokens !== null && price.outputMicrosPerMillion !== null) {
      micros +=
        (record.outputTokens * price.outputMicrosPerMillion) / 1_000_000;
      established = true;
    }
    if (established) pricedRecords += 1;
  }
  return {
    micros: pricedRecords === 0 ? null : Math.round(micros),
    pricedRecords,
    unpricedRecords: usage.length - pricedRecords,
    label:
      pricedRecords === 0
        ? "unavailable"
        : providerReported === pricedRecords
          ? "provider_reported"
          : "estimated",
  };
}

export function quotaPercent(quota: QuotaFact): number | null {
  if (quota.limit === null || quota.remaining === null || quota.limit <= 0)
    return null;
  return Math.max(0, Math.min(100, (quota.remaining / quota.limit) * 100));
}

export function formatInteger(value: number | null): string {
  return value === null
    ? "Unknown"
    : new Intl.NumberFormat("en-US").format(value);
}

export function formatCompact(value: number | null): string {
  if (value === null) return "Unknown";
  return new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

export function formatMicros(value: number | null, currency = "USD"): string {
  if (value === null) return "Unknown";
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
    maximumFractionDigits: 2,
  }).format(value / 1_000_000);
}

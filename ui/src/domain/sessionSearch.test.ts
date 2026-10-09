import { afterEach, describe, expect, it, vi } from "vitest";

import { dashboardSessions } from "../fixtures/scenarios.js";
import {
  DEFAULT_SESSION_FILTERS,
  localMidnight,
  sessionMatches,
} from "./sessionSearch.js";

afterEach(() => vi.unstubAllEnvs());

describe("session search semantics", () => {
  it("uses the local calendar boundary when local Today and UTC differ", () => {
    vi.stubEnv("TZ", "Asia/Tokyo");
    expect(localMidnight(new Date("2026-10-03T16:00:00Z"))).toBe(
      "2026-10-03T15:00:00.000Z",
    );
  });

  it("keeps local midnight through short and long daylight-saving days", () => {
    vi.stubEnv("TZ", "America/New_York");
    expect(localMidnight(new Date("2026-03-08T16:00:00Z"))).toBe(
      "2026-03-08T05:00:00.000Z",
    );
    const now = new Date("2026-11-02T04:30:00Z");
    expect(localMidnight(now)).toBe("2026-11-01T04:00:00.000Z");
    expect(now.getTime() - new Date(localMidnight(now)).getTime()).toBe(
      24.5 * 3_600_000,
    );
  });

  it("does not invent phrases across message boundaries", () => {
    const base = dashboardSessions[0]!;
    const session = {
      ...base,
      timeline: [
        { ...base.timeline[0]!, body: "indigo" },
        { ...base.timeline[0]!, id: "second-message", body: "cobalt" },
      ],
    };
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: "indigo cobalt",
      }).matched,
    ).toBe(true);
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: "indigo cobalt",
        queryMode: "phrase",
      }).matched,
    ).toBe(false);
  });

  it("matches separate Unicode words across metadata and messages, but not substrings or operators", () => {
    const base = dashboardSessions[0]!;
    const session = {
      ...base,
      title: "Café",
      timeline: [
        { ...base.timeline[0]!, body: "東京 database and a later migration" },
      ],
    };
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: "cafe 東京 migration",
      }).matched,
    ).toBe(true);
    expect(
      sessionMatches(session, { ...DEFAULT_SESSION_FILTERS, text: "café 東京" })
        .matched,
    ).toBe(true);
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: "database migration",
        queryMode: "phrase",
      }).matched,
    ).toBe(false);
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: "database OR migration",
      }).matched,
    ).toBe(false);
    expect(
      sessionMatches(session, { ...DEFAULT_SESSION_FILTERS, text: "databas" })
        .matched,
    ).toBe(false);
    expect(
      sessionMatches(session, {
        ...DEFAULT_SESSION_FILTERS,
        text: '"database":migration*',
      }).matched,
    ).toBe(true);
    expect(
      sessionMatches(session, { ...DEFAULT_SESSION_FILTERS, text: "*: ()" })
        .matched,
    ).toBe(false);
  });
});

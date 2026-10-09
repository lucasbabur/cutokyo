import { afterEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_SESSION_FILTERS } from "../domain/sessionSearch.js";
import { createTauriCommandClient } from "./tauri.js";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
  vi.clearAllMocks();
});

describe("native session search client", () => {
  it("passes the complete current search contract and recalculates machine-local Today", async () => {
    vi.useFakeTimers();
    vi.stubEnv("TZ", "Asia/Tokyo");
    vi.setSystemTime(new Date("2026-10-03T16:00:00Z"));
    const filters = {
      ...DEFAULT_SESSION_FILTERS,
      text: "database migration",
      dateRange: "today" as const,
      todayStart: "stale",
      offset: 600,
      sort: "newest" as const,
    };
    const client = createTauriCommandClient();
    await client.searchSessions(filters);
    expect(invoke).toHaveBeenLastCalledWith("search_sessions", {
      filters: { ...filters, todayStart: "2026-10-03T15:00:00.000Z" },
    });
    vi.setSystemTime(new Date("2026-10-04T16:00:00Z"));
    await client.searchSessions(filters);
    expect(invoke).toHaveBeenLastCalledWith("search_sessions", {
      filters: { ...filters, todayStart: "2026-10-04T15:00:00.000Z" },
    });
  });

  it("does not send a retained Today boundary for another date range", async () => {
    await createTauriCommandClient().searchSessions({
      ...DEFAULT_SESSION_FILTERS,
      todayStart: "stale",
    });
    expect(invoke).toHaveBeenLastCalledWith("search_sessions", {
      filters: DEFAULT_SESSION_FILTERS,
    });
  });
});

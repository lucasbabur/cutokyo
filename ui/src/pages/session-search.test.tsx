import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import type {
  CommandClient,
  SessionFilters,
  SessionSearchResponse,
} from "../contracts.js";
import { DEFAULT_SESSION_FILTERS } from "../domain/sessionSearch.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { dashboardSessions } from "../fixtures/scenarios.js";
import { SessionsPage } from "./SessionsPage.js";

function response(filters: SessionFilters): SessionSearchResponse {
  return {
    meta: {
      freshness: "complete",
      notices: [],
      generatedAt: "2026-10-04T12:00:00Z",
    },
    sessions: [
      {
        ...dashboardSessions[0]!,
        matches:
          filters.text === ""
            ? []
            : [
                {
                  source: "transcript",
                  text: "needle <script>alert('unsafe')</script> café",
                  terms: ["needle", "cafe"],
                },
              ],
      },
    ],
    total: 101,
    offset: filters.offset,
    limit: filters.limit,
    hasMore: filters.offset + filters.limit < 101,
    availableProjects: ["cutokyo-community"],
    availableBranches: ["feature/observability"],
    availableTools: ["Read"],
    availableSkills: ["cutokyo-contract"],
    availableAgents: ["desktop-builder"],
  };
}

function setup(
  search = vi.fn(async (filters: SessionFilters) => response(filters)),
) {
  const client: CommandClient = {
    ...createBrowserFixtureClient("search-resume"),
    searchSessions: search,
  };
  const tree = (id: string | null) => (
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <SessionsPage sessionId={id} />
      </AnnouncementProvider>
    </CommandProvider>
  );
  const view = render(tree(null));
  return { search, tree, view };
}

beforeEach(() => {
  sessionStorage.clear();
});
afterEach(() => {
  vi.useRealTimers();
});

async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(250);
  });
}

describe("session search", () => {
  it("shows every active additional constraint while More filters is collapsed", async () => {
    const active = {
      project: "cutokyo-community",
      branch: "feature/observability",
      tool: "Read",
      skill: "cutokyo-contract",
      agent: "desktop-builder",
    };
    sessionStorage.setItem(
      `cutokyo.session-search:${location.search}`,
      JSON.stringify({ ...DEFAULT_SESSION_FILTERS, ...active }),
    );
    const { search } = setup();
    await screen.findByText("101 sessions");
    const summary = screen.getByLabelText("Active additional filters");
    expect(summary.closest("details")?.hasAttribute("open")).toBe(false);
    for (const [field, value] of Object.entries(active)) {
      expect(summary.textContent).toContain(
        `${field[0]!.toUpperCase()}${field.slice(1)}: ${value}`,
      );
    }
    expect(search.mock.lastCall?.[0]).toMatchObject(active);
    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    await waitFor(() =>
      expect(screen.queryByLabelText("Active additional filters")).toBeNull(),
    );
  });

  it("debounces typing and labels retained results while the query is pending", async () => {
    vi.useFakeTimers();
    const { search } = setup();
    await act(async () => {});
    const field = screen.getByRole("searchbox", {
      name: "Search session content",
    });
    fireEvent.change(field, { target: { value: "needle" } });
    fireEvent.change(field, { target: { value: "needle migration" } });
    expect(search).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/Showing previous results/)).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Reconcile usage capture" }),
    ).toBeTruthy();
    await settle();
    expect(search).toHaveBeenCalledTimes(2);
    expect(search.mock.calls[1]?.[0].text).toBe("needle migration");
    expect(screen.queryByText(/Showing previous results/)).toBeNull();
  });

  it("keeps stale rows and query on errors, shows the error, and retries the current request", async () => {
    vi.useFakeTimers();
    const search = vi.fn(async (filters: SessionFilters) => response(filters));
    setup(search);
    await act(async () => {});
    search.mockRejectedValueOnce(new Error("SQLite index unavailable"));
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "needle" },
    });
    await settle();
    expect(screen.getByText("SQLite index unavailable")).toBeTruthy();
    expect(screen.getByText(/not for the current search/)).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Reconcile usage capture" }),
    ).toBeTruthy();
    expect((screen.getByRole("searchbox") as HTMLInputElement).value).toBe(
      "needle",
    );
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    await act(async () => {});
    expect(search.mock.lastCall?.[0].text).toBe("needle");
    expect(screen.queryByText("SQLite index unavailable")).toBeNull();
  });

  it("renders bounded match text as React text with safe Unicode-aware highlights", async () => {
    sessionStorage.setItem(
      `cutokyo.session-search:${location.search}`,
      JSON.stringify({ ...DEFAULT_SESSION_FILTERS, text: "needle cafe" }),
    );
    setup();
    await screen.findByText(/alert\('unsafe'\)/);
    expect(document.querySelector(".session-matches script")).toBeNull();
    expect(
      [...document.querySelectorAll(".session-matches mark")].map(
        (node) => node.textContent,
      ),
    ).toEqual(["needle", "café"]);
    expect(screen.getByText("transcript")).toBeTruthy();
  });

  it("restores query, filters, sort and page after the exact detail/back link", async () => {
    const { view, tree, search } = setup();
    await screen.findByText("101 sessions");
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "needle" },
    });
    await waitFor(() => expect(search.mock.lastCall?.[0].text).toBe("needle"));
    fireEvent.click(screen.getByRole("button", { name: "Claude Code" }));
    fireEvent.change(screen.getByLabelText("Sort"), {
      target: { value: "newest" },
    });
    fireEvent.change(screen.getByLabelText("Tool"), {
      target: { value: "Read" },
    });
    await waitFor(() => expect(search.mock.lastCall?.[0].tool).toBe("Read"));
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Next page" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(search.mock.lastCall?.[0].offset).toBe(50));
    const link = screen.getByRole("link", { name: "Reconcile usage capture" });
    expect(link.getAttribute("href")).toBe("#/sessions/session-claude-73A9");
    fireEvent.click(link);
    view.rerender(tree("session-claude-73A9"));
    const back = await screen.findByRole("link", { name: "Back to sessions" });
    expect(back.getAttribute("href")).toBe("#/sessions");
    fireEvent.click(back);
    view.rerender(tree(null));
    await waitFor(() =>
      expect(search.mock.lastCall?.[0]).toMatchObject({
        text: "needle",
        harness: "claude_code",
        sort: "newest",
        tool: "Read",
        offset: 50,
      }),
    );
    expect((screen.getByRole("searchbox") as HTMLInputElement).value).toBe(
      "needle",
    );
    expect((screen.getByLabelText("Tool") as HTMLSelectElement).value).toBe(
      "Read",
    );
    expect(
      screen
        .getByRole("button", { name: "Claude Code" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("offers clear recovery for no matches even if stored history has no project", async () => {
    const empty = vi.fn(async (filters: SessionFilters) => ({
      ...response(filters),
      sessions: [],
      total: 0,
      hasMore: false,
      availableProjects: [],
    }));
    sessionStorage.setItem(
      `cutokyo.session-search:${location.search}`,
      JSON.stringify({ ...DEFAULT_SESSION_FILTERS, text: "missing" }),
    );
    setup(empty);
    expect(
      await screen.findByText("No sessions match these filters"),
    ).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: "Clear search and filters" }),
    );
    await waitFor(() => expect(empty.mock.lastCall?.[0].text).toBe(""));
    expect(await screen.findByText("No sessions yet")).toBeTruthy();
  });
});

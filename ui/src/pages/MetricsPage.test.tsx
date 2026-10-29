import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import type { CommandClient } from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { MetricsPage, durationText } from "./MetricsPage.js";

function show(client: CommandClient) {
  return render(
    <CommandProvider client={client}>
      <MetricsPage toolName={null} />
    </CommandProvider>,
  );
}

const card = (title: string) => {
  const label = screen.getByText(title);
  const article = label.closest("article");
  if (article === null) throw new Error(`no card for ${title}`);
  return within(article);
};

afterEach(() => sessionStorage.clear());

describe("metrics", () => {
  it("shows headline metrics for the last 7 days against the week before", async () => {
    const client = createBrowserFixtureClient(null);
    const metrics = vi.spyOn(client, "getSetupMetrics");
    show(client);
    await screen.findByRole("heading", { name: "By harness" });
    expect(metrics).toHaveBeenLastCalledWith(
      expect.objectContaining({ dateRange: "7d", harness: "all" }),
    );
    // (156 + 41) failed of (5533 + 412) finished.
    expect(card("Tool error rate").getByText("3.3%")).toBeTruthy();
    // Only Claude records interrupts: 12 of 383 messages, before 19 of 410.
    expect(card("Interrupts per 100 messages").getByText("3.1")).toBeTruthy();
    expect(
      card("Interrupts per 100 messages").getByText("Previous: 4.6"),
    ).toBeTruthy();
    expect(card("Corrections per 100 messages").getByText("2.9")).toBeTruthy();
    expect(card("Cache hit rate").getByText("99.1%")).toBeTruthy();
    expect(card("Compactions per session").getByText("0.43")).toBeTruthy();
    const autonomy = card("Autonomy span");
    expect(autonomy.getByText("28s")).toBeTruthy();
    expect(autonomy.getByText("3m 14s")).toBeTruthy();
  });

  it("links repeat failures to their tool samples and lists unused items", async () => {
    show(createBrowserFixtureClient(null));
    const failures = await screen.findByRole("region", {
      name: "Repeat failures",
    });
    expect(
      within(failures)
        .getByRole("link", { name: "herdr-progress" })
        .getAttribute("href"),
    ).toBe("#/metrics/Bash");
    const servers = screen.getByRole("region", { name: "MCP servers" });
    const ephra = within(servers).getByRole("row", { name: /ephra-tools/ });
    expect(within(ephra).getByText("14.1%")).toBeTruthy();
    expect(
      screen.getByRole("region", { name: /^Unused skills · \d+$/ }),
    ).toBeTruthy();
    expect(
      screen.getByRole("region", { name: "Projects" }).textContent,
    ).toContain("No project");
  });

  it("keeps unknown harness facts unknown and drops the comparison for any time", async () => {
    show(createBrowserFixtureClient(null));
    await screen.findByRole("heading", { name: "By harness" });
    await userEvent.click(screen.getByRole("button", { name: /Codex/ }));
    expect(
      await card("Interrupts per 100 messages").findByText("—"),
    ).toBeTruthy();
    expect(card("Compactions per session").getByText("—")).toBeTruthy();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Date" }),
      "all",
    );
    await waitFor(() => expect(screen.queryByText(/^Previous:/)).toBeNull());
  });

  it("shows an empty state without history", async () => {
    show(createBrowserFixtureClient("empty-history"));
    expect(await screen.findByText("No activity")).toBeTruthy();
  });

  it("formats autonomy spans", () => {
    expect(durationText(null)).toBe("—");
    expect(durationText(45)).toBe("45s");
    expect(durationText(120)).toBe("2m");
    expect(durationText(194)).toBe("3m 14s");
    expect(durationText(3900)).toBe("1h 5m");
  });
});

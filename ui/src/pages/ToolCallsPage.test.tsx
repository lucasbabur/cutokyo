import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import type { CommandClient } from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import {
  ToolCallsPage,
  inputPreview,
  payloadText,
  percent,
} from "./ToolCallsPage.js";

function show(client: CommandClient, toolName: string | null = null) {
  return render(
    <CommandProvider client={client}>
      <ToolCallsPage toolName={toolName} />
    </CommandProvider>,
  );
}

const outcomes = () => within(screen.getByRole("group", { name: "Outcome" }));

afterEach(() => sessionStorage.clear());

describe("tool calls", () => {
  it("lists every tool with its call count, success rate and failures", async () => {
    show(createBrowserFixtureClient(null));
    const table = await screen.findByRole("table");
    const bash = within(table).getByRole("row", { name: /^Bash/ });
    // 40 calls, every seventh starting at index 1 fails: 6 failures.
    expect(within(bash).getByText("40")).toBeTruthy();
    expect(within(bash).getByText("85.0%")).toBeTruthy();
    expect(within(bash).getByText("6")).toBeTruthy();
    expect(
      within(bash).getByRole("link", { name: "Bash" }).getAttribute("href"),
    ).toBe("#/tools/Bash");
    expect(
      screen.getByRole("heading", {
        name: "74 calls · 83.8% succeeded · 12 failed",
      }),
    ).toBeTruthy();
  });

  it("filters by name, sorts by failures and by harness", async () => {
    const client = createBrowserFixtureClient(null);
    const stats = vi.spyOn(client, "getToolCallStats");
    show(client);
    await screen.findByRole("table");
    await userEvent.type(
      screen.getByRole("searchbox", { name: "Filter tools by name" }),
      "sql",
    );
    expect(screen.getAllByRole("row")).toHaveLength(2);
    expect(
      screen.getByRole("link", { name: "mcp__cutokyo__cutokyo_sql_query" }),
    ).toBeTruthy();
    await userEvent.clear(
      screen.getByRole("searchbox", { name: "Filter tools by name" }),
    );
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Sort" }),
      "failures",
    );
    const rows = screen
      .getAllByRole("rowheader")
      .map((cell) => cell.textContent);
    expect(rows.slice(0, 2)).toEqual(["Bash", "shell"]);
    await userEvent.click(screen.getByRole("button", { name: /Codex/ }));
    await waitFor(() =>
      expect(
        screen.getAllByRole("rowheader").map((cell) => cell.textContent),
      ).toEqual(["shell"]),
    );
    expect(stats).toHaveBeenLastCalledWith(
      expect.objectContaining({ harness: "codex" }),
    );
  });

  it("opens on failures, shows input and output, and switches outcome", async () => {
    show(createBrowserFixtureClient(null), "Bash");
    expect(
      await screen.findByRole("heading", {
        name: "6 failed calls, newest first",
      }),
    ).toBeTruthy();
    expect(
      outcomes().getByRole("button", { name: /Failed/, pressed: true }),
    ).toBeTruthy();
    const samples = screen.getAllByRole("listitem");
    expect(samples).toHaveLength(6);
    const first = samples[0]!;
    expect(within(first).getByText("Failed")).toBeTruthy();
    expect(
      within(first).getByText(/error: test store::tool_calls/),
    ).toBeTruthy();
    expect(
      within(first)
        .getByRole("link", { name: /Reconcile usage capture/ })
        .getAttribute("href"),
    ).toBe("#/sessions/session-claude-73A9");

    await userEvent.click(outcomes().getByRole("button", { name: /^All/ }));
    expect(
      await screen.findByRole("heading", { name: "40 calls, newest first" }),
    ).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(25);
    await userEvent.click(screen.getByRole("button", { name: "Load 15 more" }));
    await waitFor(() =>
      expect(screen.getAllByRole("listitem")).toHaveLength(40),
    );
    expect(screen.queryByRole("button", { name: /more$/ })).toBeNull();
    const ids = new Set(
      screen.getAllByRole("listitem").map((item) => item.textContent),
    );
    expect(ids.size).toBe(40);
  });

  it("opens on all calls for a tool that never failed", async () => {
    show(createBrowserFixtureClient(null), "mcp__cutokyo__cutokyo_sql_query");
    expect(
      await screen.findByRole("heading", { name: "4 calls, newest first" }),
    ).toBeTruthy();
    expect(
      outcomes().getByRole("button", { name: /^All/, pressed: true }),
    ).toBeTruthy();
  });

  it("says when no tool calls are captured", async () => {
    show(createBrowserFixtureClient("empty-history"));
    expect(await screen.findByText("No tool calls")).toBeTruthy();
  });

  it("keeps harness and date filters between the list and a tool", async () => {
    const client = createBrowserFixtureClient(null);
    const view = show(client);
    await screen.findByRole("table");
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Date" }),
      "7d",
    );
    view.unmount();
    const samples = vi.spyOn(client, "getToolCallSamples");
    show(client, "Read");
    await screen.findByRole("heading", { name: /failed calls/ });
    expect(samples).toHaveBeenCalledWith(
      expect.objectContaining({
        filters: expect.objectContaining({ dateRange: "7d" }),
      }),
    );
  });
});

describe("tool call formatting", () => {
  it("previews the meaningful input field", () => {
    expect(inputPreview({ command: "ls -la", description: "list" })).toBe(
      "ls -la",
    );
    expect(inputPreview({ command: ["/bin/bash", "-lc", "rg x"] })).toBe(
      "/bin/bash -lc rg x",
    );
    expect(inputPreview({ file_path: "/a.rs" })).toBe("/a.rs");
    expect(inputPreview({ other: 1 })).toBe('{"other":1}');
    expect(inputPreview(null)).toBe("No input recorded");
  });

  it("prints text output raw and structured output as JSON", () => {
    expect(payloadText("line one\nline two")).toBe("line one\nline two");
    expect(payloadText({ ok: true })).toBe('{\n  "ok": true\n}');
    expect(payloadText(null)).toBe("");
  });

  it("never rounds a mix of outcomes to 100% or 0%", () => {
    expect(percent(0, 0)).toBe("—");
    expect(percent(1, 1)).toBe("100%");
    expect(percent(9_999, 10_000)).toBe(">99.9%");
    expect(percent(1, 10_000)).toBe("<0.1%");
    expect(percent(9_990, 10_000)).toBe("99.9%");
    expect(percent(0, 5)).toBe("0%");
  });
});

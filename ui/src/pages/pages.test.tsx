import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import { AppShell } from "../components/AppShell.js";
import type { CommandClient } from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { DashboardPage } from "./DashboardPage.js";
import { HealthPage } from "./HealthPage.js";
import { OnboardingPage } from "./OnboardingPage.js";
import { SessionsPage } from "./SessionsPage.js";

function renderWithClient(client: CommandClient, node: ReactNode) {
  return render(
    <CommandProvider client={client}>
      <AnnouncementProvider>{node}</AnnouncementProvider>
    </CommandProvider>,
  );
}

describe("desktop route components", () => {
  it("moves focus to a route heading that renders after loading", async () => {
    const client = createBrowserFixtureClient("health-degraded");
    const bootstrap = await client.getBootstrap();
    const shell = (path: "/dashboard" | "/health") => (
      <CommandProvider client={client}>
        <AnnouncementProvider>
          <AppShell bootstrap={bootstrap} currentPath={path}>
            {path === "/health" ? <HealthPage /> : <DashboardPage />}
          </AppShell>
        </AnnouncementProvider>
      </CommandProvider>
    );
    const view = render(shell("/dashboard"));
    await screen.findByRole("heading", { name: "Overview" });
    expect(document.activeElement).toBe(document.body);
    view.rerender(shell("/health"));
    const heading = await screen.findByRole("heading", {
      name: "System health",
    });
    await waitFor(() => expect(document.activeElement).toBe(heading));
  });

  it("renders chart totals, raw values, uncertainty, and absent context consistently", async () => {
    renderWithClient(
      createBrowserFixtureClient("populated-dashboard"),
      <DashboardPage />,
    );
    await screen.findByRole("heading", { name: "Overview" });
    const table = screen.getByRole("table", {
      name: "Raw values used by the token usage chart",
    });
    expect(within(table).getByText("184,200")).toBeTruthy();
    expect(within(table).getByText("42,810")).toBeTruthy();
    expect(within(table).getByText("227,010")).toBeTruthy();
    expect(screen.getByText("Absent, not zero")).toBeTruthy();
    const quota = screen.getByRole("meter", {
      name: "Claude 5-hour window remaining",
    });
    expect(quota.getAttribute("aria-valuenow")).toBe("72");
    expect(screen.getByText("Remaining amount unknown")).toBeTruthy();
  });

  it("shows a useful history-empty state instead of fake zero analytics", async () => {
    renderWithClient(
      createBrowserFixtureClient("empty-history"),
      <SessionsPage sessionId={null} />,
    );
    await screen.findByRole("heading", { name: "Sessions" });
    expect(await screen.findByText("History is empty—not zero")).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Review capture setup" }),
    ).toBeTruthy();
  });

  it("searches one session and exposes its coverage label", async () => {
    const user = userEvent.setup();
    renderWithClient(
      createBrowserFixtureClient("search-resume"),
      <SessionsPage sessionId={null} />,
    );
    const search = await screen.findByRole("searchbox", {
      name: "Search session content",
    });
    await user.type(search, "JEV exact resume needle 73A9");
    await waitFor(() => {
      expect(screen.getByText("1 session")).toBeTruthy();
    });
    expect(
      screen.getByRole("link", { name: "Reconcile usage capture" }),
    ).toBeTruthy();
    expect(screen.getByText("complete")).toBeTruthy();
  });

  it("moves focus to content search with the advertised keyboard shortcut", async () => {
    const user = userEvent.setup();
    renderWithClient(
      createBrowserFixtureClient("search-resume"),
      <SessionsPage sessionId={null} />,
    );
    const search = await screen.findByRole("searchbox", {
      name: "Search session content",
    });
    const harness = screen.getByRole("combobox", { name: "Harness" });
    harness.focus();
    await user.keyboard("{Control>}k{/Control}");
    expect(document.activeElement).toBe(search);
    expect(search.getAttribute("aria-keyshortcuts")).toBe("Control+K Meta+K");
  });

  it("confirms the exact native target and restores focus after cancel", async () => {
    const user = userEvent.setup();
    renderWithClient(
      createBrowserFixtureClient("search-resume"),
      <SessionsPage sessionId="session-claude-73A9" />,
    );
    const resume = await screen.findByRole("button", { name: "Resume" });
    resume.focus();
    await user.click(resume);
    const dialog = await screen.findByRole("dialog", {
      name: "Resume this exact native session?",
    });
    expect(await within(dialog).findByText("claude-jev-73A9")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(document.activeElement).toBe(resume);
  });

  it("traps keyboard focus inside destructive confirmation and closes on Escape", async () => {
    const user = userEvent.setup();
    renderWithClient(
      createBrowserFixtureClient("search-resume"),
      <SessionsPage sessionId="session-claude-73A9" />,
    );
    const trigger = await screen.findByRole("button", {
      name: "Delete session",
    });
    trigger.focus();
    await user.click(trigger);
    const dialog = await screen.findByRole("dialog", {
      name: "Delete only this session?",
    });
    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    const confirm = within(dialog).getByRole("button", {
      name: "Delete selected session",
    });
    await waitFor(() => {
      expect((confirm as HTMLButtonElement).disabled).toBe(false);
    });
    confirm.focus();
    await user.tab();
    expect(document.activeElement).toBe(
      within(dialog).getByRole("button", { name: "Close dialog" }),
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(cancel).not.toBe(document.activeElement);
  });

  it("requires the plaintext boundary and keeps optional egress off during onboarding", async () => {
    const client = createBrowserFixtureClient("onboarding-empty");
    const user = userEvent.setup();
    renderWithClient(client, <OnboardingPage onComplete={() => undefined} />);
    const complete = await screen.findByRole("button", {
      name: /Complete local-only setup/,
    });
    expect((complete as HTMLButtonElement).disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /I understand/ }));
    expect((complete as HTMLButtonElement).disabled).toBe(false);
    await user.click(complete);
    await waitFor(async () => {
      expect((await client.getBootstrap()).onboardingComplete).toBe(true);
    });
    expect((await client.getGuards()).proxyEnabled).toBe(false);
  });

  it("offers a retry for command-boundary errors", async () => {
    const base = createBrowserFixtureClient("populated-dashboard");
    const client: CommandClient = {
      ...base,
      getDashboard: async () => {
        throw new Error("Synthetic dashboard failure");
      },
    };
    renderWithClient(client, <DashboardPage />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Synthetic dashboard failure");
    expect(screen.getByText("Synthetic dashboard failure")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });
});

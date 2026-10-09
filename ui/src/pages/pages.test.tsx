import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import { AppShell } from "../components/AppShell.js";
import type { ActionReceipt, CommandClient } from "../contracts.js";
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
  it("discloses the exact bounded scope of overview totals", async () => {
    const base = createBrowserFixtureClient("populated-dashboard");
    const dashboard = await base.getDashboard();
    const notice =
      "Overview totals cover the latest 500 of 503 stored sessions. Sessions search can reach all retained history.";
    const bootstrap = await base.getBootstrap();
    renderWithClient(
      {
        ...base,
        getDashboard: async () => ({
          ...dashboard,
          meta: { ...dashboard.meta, freshness: "partial", notices: [notice] },
        }),
      },
      <AppShell bootstrap={bootstrap} currentPath="/dashboard">
        <DashboardPage />
      </AppShell>,
    );
    const trigger = await screen.findByRole("button", { name: /Warnings/ });
    await waitFor(() => expect(trigger.textContent).toContain("1"));
    await userEvent.click(trigger);
    const panel = screen.getByRole("region", { name: "Warnings for Overview" });
    expect(panel.textContent).toContain(notice);
    expect(panel.textContent).toContain("Partial data");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: /Warnings for/ })).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

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
      name: "Health",
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
    expect(screen.queryByText("Absent, not zero")).toBeNull();
    expect(screen.queryByText("Request composition")).toBeNull();
    expect(screen.queryByText("OBSERVED", { exact: false })).toBeNull();
    const quota = screen.getByRole("meter", {
      name: "Claude 5-hour window remaining",
    });
    expect(quota.getAttribute("aria-valuenow")).toBe("72");
    expect(screen.getByText("Unknown")).toBeTruthy();
  });

  it("shows a useful history-empty state instead of fake zero analytics", async () => {
    renderWithClient(
      createBrowserFixtureClient("empty-history"),
      <SessionsPage sessionId={null} />,
    );
    await screen.findByRole("heading", { name: "Sessions" });
    expect(await screen.findByText("No sessions yet")).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Review capture setup" }),
    ).toBeTruthy();
  });

  it("searches one session without a per-row coverage badge", async () => {
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
    expect(screen.queryByText("complete")).toBeNull();
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
    const harness = screen.getByRole("button", { name: "Codex" });
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
      name: "Resume in Claude Code?",
    });
    expect(await within(dialog).findByText(/^Folder:/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(document.activeElement).toBe(resume);
  });

  it("keeps a failed resume explanation inside the active confirmation and retries once", async () => {
    const base = createBrowserFixtureClient("search-resume");
    let release: ((receipt: ActionReceipt) => void) | undefined;
    const retry = new Promise<ActionReceipt>((resolve) => {
      release = resolve;
    });
    const resumeSession = vi
      .fn(base.resumeSession)
      .mockRejectedValueOnce(
        new Error("Terminal startup failed. Retry resume."),
      )
      .mockImplementationOnce(() => retry);
    const preview = await base.previewResume("session-claude-73A9");
    const command = "claude --resume claude-jev-73A9";
    const user = userEvent.setup();
    renderWithClient(
      {
        ...base,
        resumeSession,
        previewResume: async () => ({
          ...preview,
          commandDescription: command,
          projectDirectory: "/synthetic/project with spaces",
          projectContextKnown: true,
        }),
      },
      <SessionsPage sessionId="session-claude-73A9" />,
    );
    const trigger = await screen.findByRole("button", { name: "Resume" });
    await user.click(trigger);
    const dialog = await screen.findByRole("dialog", {
      name: "Resume in Claude Code?",
    });
    await within(dialog).findByText(/^Folder:/);
    await user.click(
      await within(dialog).findByRole("button", {
        name: "Resume",
      }),
    );
    expect((await within(dialog).findByRole("alert")).textContent).toContain(
      "Terminal startup failed. Retry resume.",
    );
    expect(screen.getByRole("status").textContent).toBe("");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(
      within(dialog).getByText("Folder: /synthetic/project with spaces"),
    ).toBeTruthy();
    expect(within(dialog).queryByText(command)).toBeNull();
    const confirm = within(dialog).getByRole("button", {
      name: "Resume",
    });
    expect((confirm as HTMLButtonElement).disabled).toBe(false);
    act(() => {
      fireEvent.click(confirm);
      fireEvent.click(confirm);
    });
    expect(resumeSession).toHaveBeenCalledTimes(2);
    expect(resumeSession).toHaveBeenNthCalledWith(2, "session-claude-73A9");
    expect(within(dialog).queryByRole("alert")).toBeNull();
    for (const name of ["Opening…", "Cancel", "Close dialog"]) {
      expect(
        (within(dialog).getByRole("button", { name }) as HTMLButtonElement)
          .disabled,
      ).toBe(true);
    }
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBe(dialog);
    const message =
      "Claude Code started in a visible terminal with exact native ID claude-jev-73A9. Check that terminal for session activation or login errors; Cutokyo cannot verify native session activation.";
    await act(async () => {
      release?.({ ok: true, status: "warning", message });
      await retry;
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(
      screen.getByText(message, { selector: ".success-message p" }),
    ).toBeTruthy();
    expect(document.activeElement).toBe(trigger);
  });

  it.each(["Cancel", "Close dialog", "Escape"])(
    "dismisses a failed resume with %s, restores focus and clears the next preview",
    async (close) => {
      const base = createBrowserFixtureClient("search-resume");
      const resumeSession = vi
        .fn(base.resumeSession)
        .mockRejectedValue(new Error("Terminal startup failed. Retry resume."));
      const user = userEvent.setup();
      renderWithClient(
        { ...base, resumeSession },
        <SessionsPage sessionId="session-claude-73A9" />,
      );
      const trigger = await screen.findByRole("button", { name: "Resume" });
      await user.click(trigger);
      const dialog = await screen.findByRole("dialog");
      await within(dialog).findByText(/^Folder:/);
      await user.click(
        await within(dialog).findByRole("button", {
          name: "Resume",
        }),
      );
      await within(dialog).findByRole("alert");
      if (close === "Escape") await user.keyboard("{Escape}");
      else
        await user.click(within(dialog).getByRole("button", { name: close }));
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(document.activeElement).toBe(trigger);
      expect(screen.queryByRole("alert")).toBeNull();
      await user.click(trigger);
      const next = await screen.findByRole("dialog");
      await within(next).findByText(/^Folder:/);
      expect(within(next).queryByRole("alert")).toBeNull();
      expect(screen.getByRole("status").textContent).toBe("");
      expect(resumeSession).toHaveBeenCalledTimes(1);
    },
  );

  it.each(["unavailable", "cancelled"] as const)(
    "keeps a resolved %s resume receipt inside confirmation without false success",
    async (status) => {
      const base = createBrowserFixtureClient("search-resume");
      const message = "No interactive harness was started.";
      const user = userEvent.setup();
      renderWithClient(
        {
          ...base,
          resumeSession: async () => ({ ok: false, status, message }),
        },
        <SessionsPage sessionId="session-claude-73A9" />,
      );
      await user.click(await screen.findByRole("button", { name: "Resume" }));
      const dialog = await screen.findByRole("dialog");
      await within(dialog).findByText(/^Folder:/);
      await user.click(
        await within(dialog).findByRole("button", {
          name: "Resume",
        }),
      );
      expect((await within(dialog).findByRole("alert")).textContent).toBe(
        message,
      );
      expect(screen.getByRole("status").textContent).toBe("");
      expect(
        (
          within(dialog).getByRole("button", {
            name: "Resume",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false);
    },
  );

  it("keeps resume preview failures recoverable without running the command", async () => {
    const base = createBrowserFixtureClient("search-resume");
    const previewResume = vi
      .fn(base.previewResume)
      .mockRejectedValueOnce(new Error("Exact native target is unavailable."));
    const resumeSession = vi.fn(base.resumeSession);
    const user = userEvent.setup();
    renderWithClient(
      { ...base, previewResume, resumeSession },
      <SessionsPage sessionId="session-claude-73A9" />,
    );
    const trigger = await screen.findByRole("button", { name: "Resume" });
    await user.click(trigger);
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Exact native target is unavailable.",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    await user.click(trigger);
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findByText(/^Folder:/);
    expect(within(dialog).queryByRole("alert")).toBeNull();
    expect(resumeSession).not.toHaveBeenCalled();
    expect(previewResume).toHaveBeenNthCalledWith(2, "session-claude-73A9");
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

  it("previews and cancels capture without applying native changes", async () => {
    const base = createBrowserFixtureClient("onboarding-empty");
    const apply = vi.fn(base.applyCaptureSetup);
    const user = userEvent.setup();
    renderWithClient(
      { ...base, applyCaptureSetup: apply },
      <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
    );
    await user.click(
      await screen.findByRole("button", {
        name: "More actions for Claude Code",
      }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Install Claude Code" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Install Claude Code?",
    });
    expect(within(dialog).getByLabelText("Files that change")).toBeTruthy();
    expect(apply).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    expect((await base.getBootstrap()).onboardingComplete).toBe(false);
  });

  it("confirms one pending setup token once and only continues after verified capture", async () => {
    const base = createBrowserFixtureClient("onboarding-empty");
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const apply = vi.fn(async (token: string) => {
      await pending;
      return base.applyCaptureSetup(token);
    });
    const complete = vi.fn(base.completeOnboarding);
    const user = userEvent.setup();
    renderWithClient(
      { ...base, applyCaptureSetup: apply, completeOnboarding: complete },
      <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
    );
    await user.click(
      await screen.findByRole("checkbox", { name: /stored unencrypted/ }),
    );
    await user.click(
      screen.getByRole("checkbox", { name: "Capture Claude Code" }),
    );
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Install selected" }));
    const confirm = await screen.findByRole("button", { name: "Confirm" });
    act(() => {
      fireEvent.click(confirm);
      fireEvent.click(confirm);
    });
    expect(apply).toHaveBeenCalledTimes(1);
    expect(complete).not.toHaveBeenCalled();
    await act(async () => {
      release?.();
      await pending;
    });
    const next = await screen.findByRole("button", { name: "Continue" });
    await waitFor(() =>
      expect((next as HTMLButtonElement).disabled).toBe(false),
    );
    await user.click(next);
    await waitFor(() => {
      expect(complete).toHaveBeenCalledTimes(1);
    });
    expect(complete).toHaveBeenCalledWith({
      mode: "install",
      harnesses: ["claude_code"],
      acknowledgedPlaintextStorage: true,
      proxyEnabled: false,
      analysisEgressEnabled: false,
    });
  });

  it("marks each agent Ready only after its configuration is verified", async () => {
    const base = createBrowserFixtureClient("onboarding-empty");
    const user = userEvent.setup();
    renderWithClient(
      base,
      <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
    );
    await user.click(
      await screen.findByRole("checkbox", { name: /stored unencrypted/ }),
    );
    for (const name of ["Claude Code", "Codex", "OpenCode"]) {
      const row = screen.getByRole("listitem", { name: `${name} capture` });
      expect(within(row).queryByText("Ready")).toBeNull();
      await user.click(within(row).getByRole("checkbox"));
    }
    await user.click(screen.getByRole("button", { name: "Install selected" }));
    for (const name of ["Claude Code", "Codex", "OpenCode"]) {
      await user.click(await screen.findByRole("button", { name: "Confirm" }));
      await waitFor(() =>
        expect(
          within(
            screen.getByRole("listitem", { name: `${name} capture` }),
          ).getByText("Ready"),
        ).toBeTruthy(),
      );
    }
    expect(
      await screen.findByRole("button", { name: "Continue" }),
    ).toBeTruthy();
  });

  it("keeps unavailable recovery inline and permits an explicit retry", async () => {
    const base = createBrowserFixtureClient("onboarding-empty");
    const preview = vi
      .fn(base.previewCaptureSetup)
      .mockRejectedValueOnce(
        new Error(
          "Native harness missing. Install the supported version or browse without installing.",
        ),
      );
    const user = userEvent.setup();
    renderWithClient(
      { ...base, previewCaptureSetup: preview },
      <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
    );
    const repair = async () => {
      await user.click(
        await screen.findByRole("button", { name: "More actions for Codex" }),
      );
      await user.click(screen.getByRole("menuitem", { name: "Repair Codex" }));
    };
    await repair();
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Native harness missing",
    );
    await repair();
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(preview).toHaveBeenNthCalledWith(2, "codex", "recover");
  });

  it("requires the storage acknowledgement before skipping, and keeps optional egress off", async () => {
    const client = createBrowserFixtureClient("onboarding-empty");
    const user = userEvent.setup();
    renderWithClient(
      client,
      <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
    );
    const skip = await screen.findByRole("button", { name: "Skip for now" });
    expect((skip as HTMLButtonElement).disabled).toBe(true);
    await user.click(
      screen.getByRole("checkbox", { name: /stored unencrypted/ }),
    );
    expect((skip as HTMLButtonElement).disabled).toBe(false);
    await user.click(skip);
    await waitFor(async () => {
      expect((await client.getBootstrap()).onboardingComplete).toBe(true);
    });
    expect((await client.getProxyStatus()).proxyEnabled).toBe(false);
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

  it("keeps coverage as plain text inside session details, not a badge", async () => {
    renderWithClient(
      createBrowserFixtureClient("search-resume"),
      <SessionsPage sessionId="session-claude-73A9" />,
    );
    await screen.findByRole("heading", { name: "Timeline" });
    expect(screen.getAllByText("Coverage").length).toBeGreaterThan(0);
    expect(document.querySelector(".status-pill--complete")).toBeNull();
  });
});

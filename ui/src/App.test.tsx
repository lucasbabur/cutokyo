import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { App } from "./App.js";
import { CommandProvider } from "./commands/context.js";
import { AnnouncementProvider } from "./components/Announcer.js";
import { createBrowserFixtureClient } from "./fixtures/browserAdapter.js";

async function openWarnings(
  user: ReturnType<typeof userEvent.setup>,
  expected: string,
) {
  const trigger = screen.getByRole("button", { name: /Warnings/ });
  if (trigger.getAttribute("aria-expanded") !== "true")
    await user.click(trigger);
  expect(
    screen.getByRole("region", { name: /^Warnings for/ }).textContent,
  ).toContain(expected);
}

async function recheck(user: ReturnType<typeof userEvent.setup>) {
  const trigger = screen.getByRole("button", { name: /^Warnings/ });
  if (trigger.getAttribute("aria-expanded") !== "true")
    await user.click(trigger);
  await user.click(
    screen.getByRole("button", { name: "Recheck local status" }),
  );
}

describe("application ownership state", () => {
  it("shows desktop decode warnings once and rechecks repaired preferences without clearing other notices", async () => {
    globalThis.location.hash = "#/onboarding";
    const base = createBrowserFixtureClient("populated-dashboard");
    const initial = await base.getBootstrap();
    const onboarding = await base.getOnboarding();
    const decodeNotice =
      "Desktop settings could not be decoded and were not overwritten: unknown field `mcp_enabled`.";
    const writerNotice = "Another process owns the history writer.";
    const captureNotice = "Capture coverage remains partial.";
    let repaired = false;
    const getBootstrap = vi.fn(async () => ({
      ...initial,
      onboardingComplete: repaired,
      writerMode: "read_only" as const,
      startupNotice: repaired
        ? writerNotice
        : `${decodeNotice}\n${writerNotice}`,
    }));
    const getOnboarding = vi.fn(async () => ({
      ...onboarding,
      complete: repaired,
      meta: { ...onboarding.meta, notices: [captureNotice] },
    }));
    const user = userEvent.setup();
    render(
      <CommandProvider client={{ ...base, getBootstrap, getOnboarding }}>
        <AnnouncementProvider>
          <App />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await screen.findByRole("heading", {
      name: "Set up Cutokyo",
    });
    await openWarnings(user, "Desktop settings could not be decoded");
    expect(
      screen.getAllByText(/Desktop settings could not be decoded/),
    ).toHaveLength(1);
    await openWarnings(user, captureNotice);
    repaired = true;
    await recheck(user);
    await screen.findByRole("heading", { name: "Manage capture" });
    await waitFor(() => expect(getOnboarding).toHaveBeenCalledTimes(2));
    expect(getBootstrap).toHaveBeenCalledTimes(2);
    expect(
      screen.queryByText(/Desktop settings could not be decoded/),
    ).toBeNull();
    expect(screen.getByText(writerNotice)).toBeTruthy();
    await openWarnings(user, captureNotice);
    await openWarnings(user, "Another Cutokyo window");
  });

  it("retains the setup draft and warning when a status recheck fails and permits retry", async () => {
    globalThis.location.hash = "#/onboarding";
    const base = createBrowserFixtureClient("populated-dashboard");
    const initial = await base.getBootstrap();
    const warning =
      "Desktop settings could not be decoded and were not overwritten.";
    const response = {
      ...initial,
      onboardingComplete: false,
      startupNotice: warning,
    };
    const getBootstrap = vi
      .fn()
      .mockResolvedValueOnce(response)
      .mockRejectedValueOnce(new Error("Local state could not be read."))
      .mockResolvedValue(response);
    const user = userEvent.setup();
    render(
      <CommandProvider client={{ ...base, getBootstrap }}>
        <AnnouncementProvider>
          <App />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    const selection = await screen.findByRole("checkbox", {
      name: /Claude Code/,
    });
    await user.click(selection);
    await recheck(user);
    const failure = await screen.findByRole("alert");
    expect(failure.textContent).toContain("Local status could not refresh");
    expect(failure.textContent).toContain("Local state could not be read.");
    await openWarnings(user, warning);
    expect((selection as HTMLInputElement).checked).toBe(true);
    await user.click(within(failure).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(getBootstrap).toHaveBeenCalledTimes(3);
    expect((selection as HTMLInputElement).checked).toBe(true);
    await openWarnings(user, warning);
  });

  it.each(["settings", "onboarding"] as const)(
    "discloses a failed %s refresh even when bootstrap succeeds and retains the page for retry",
    async (page) => {
      globalThis.location.hash = `#/${page}`;
      const base = createBrowserFixtureClient("populated-dashboard");
      const initial = await base.getBootstrap();
      const warning = "Another process owns the history writer.";
      const getBootstrap = vi.fn(async () => ({
        ...initial,
        onboardingComplete: page === "settings",
        startupNotice: warning,
      }));
      const getSettings = vi
        .fn()
        .mockResolvedValueOnce(await base.getSettings())
        .mockRejectedValueOnce(new Error("Settings read failed."))
        .mockImplementation(base.getSettings);
      const getOnboarding = vi
        .fn()
        .mockResolvedValueOnce(await base.getOnboarding())
        .mockRejectedValueOnce(new Error("Setup read failed."))
        .mockImplementation(base.getOnboarding);
      const user = userEvent.setup();
      render(
        <CommandProvider
          client={{ ...base, getBootstrap, getSettings, getOnboarding }}
        >
          <AnnouncementProvider>
            <App />
          </AnnouncementProvider>
        </CommandProvider>,
      );
      const headingName = page === "settings" ? "Settings" : "Set up Cutokyo";
      await screen.findByRole("heading", { name: headingName });
      const choice =
        page === "onboarding"
          ? screen.getByRole("checkbox", { name: /Claude Code/ })
          : null;
      if (choice !== null) await user.click(choice);
      await recheck(user);
      const failure = await screen.findByRole("alert");
      expect(failure.textContent).toContain(
        page === "settings"
          ? "Settings could not refresh"
          : "Setup could not refresh",
      );
      expect(screen.getByRole("heading", { name: headingName })).toBeTruthy();
      await openWarnings(user, warning);
      if (choice !== null)
        expect((choice as HTMLInputElement).checked).toBe(true);
      await user.click(within(failure).getByRole("button", { name: "Retry" }));
      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
      expect(
        page === "settings" ? getSettings : getOnboarding,
      ).toHaveBeenCalledTimes(3);
      expect(getBootstrap).toHaveBeenCalledTimes(2);
      if (choice !== null)
        expect((choice as HTMLInputElement).checked).toBe(true);
    },
  );

  it("lets a completed user with history cancel and remove native capture through Settings without resetting preferences", async () => {
    globalThis.location.hash = "#/settings";
    const base = createBrowserFixtureClient("populated-dashboard");
    const installed = await base.previewCaptureSetup("claude_code", "install");
    await base.applyCaptureSetup(installed.previewToken);
    const settings = await base.getSettings();
    const selectionIntent = await base.getOnboarding();
    expect(selectionIntent.complete).toBe(true);
    const history = await base.getDashboard();
    expect(history.sessions.length).toBeGreaterThan(0);
    const complete = vi.fn(base.completeOnboarding);
    const patch = vi.fn(base.patchSettings);
    const preferences = vi.fn(base.patchDesktopPreferences);
    const apply = vi.fn(base.applyCaptureSetup);
    const user = userEvent.setup();
    render(
      <CommandProvider
        client={{
          ...base,
          completeOnboarding: complete,
          patchSettings: patch,
          patchDesktopPreferences: preferences,
          applyCaptureSetup: apply,
        }}
      >
        <AnnouncementProvider>
          <App />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await user.click(
      await screen.findByRole("link", { name: "Manage capture" }),
    );
    await screen.findByRole("heading", { name: "Manage capture" });
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Skip for now" })).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Claude Code/ })).toBeNull();
    await user.click(
      screen.getByRole("button", { name: "More actions for Claude Code" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Remove Claude Code" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Remove Claude Code?",
    });
    expect(apply).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "More actions for Claude Code" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Remove Claude Code" }),
    );
    await user.click(await screen.findByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(apply).toHaveBeenCalledTimes(1);
    const verify = await base.previewCaptureSetup("claude_code", "install");
    expect(verify.verified).toBe(false);
    expect(await base.getOnboarding()).toEqual(selectionIntent);
    expect(await base.getSettings()).toEqual(settings);
    expect(await base.getDashboard()).toEqual(history);
    expect((await base.getBootstrap()).onboardingComplete).toBe(true);
    expect(complete).not.toHaveBeenCalled();
    expect(patch).not.toHaveBeenCalled();
    expect(preferences).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Return to Settings" }),
    );
    await screen.findByRole("heading", { name: "Settings" });
    expect(globalThis.location.hash).toBe("#/settings");
  });

  it("keeps completed-user removal pending once, reports failure, and permits recovery without a stale verification claim", async () => {
    globalThis.location.hash = "#/onboarding";
    const base = createBrowserFixtureClient("populated-dashboard");
    const installed = await base.previewCaptureSetup("claude_code", "install");
    await base.applyCaptureSetup(installed.previewToken);
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const apply = vi.fn(async (_token: string) => {
      await pending;
      throw new Error(
        "Native configuration changed. Preview again before removing capture.",
      );
    });
    const complete = vi.fn(base.completeOnboarding);
    const user = userEvent.setup();
    render(
      <CommandProvider
        client={{
          ...base,
          applyCaptureSetup: apply,
          completeOnboarding: complete,
        }}
      >
        <AnnouncementProvider>
          <App />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await screen.findByRole("heading", { name: "Manage capture" });
    await user.click(
      screen.getByRole("button", { name: "More actions for Claude Code" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Remove Claude Code" }),
    );
    const confirm = await screen.findByRole("button", { name: "Confirm" });
    act(() => {
      fireEvent.click(confirm);
      fireEvent.click(confirm);
    });
    expect(apply).toHaveBeenCalledTimes(1);
    expect(
      (
        screen.getByRole("button", {
          name: "Return to Settings",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "More actions for Codex",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await act(async () => {
      release?.();
      await pending;
    });
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Native configuration changed",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(
      screen.getByRole("button", { name: "More actions for Claude Code" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Repair Claude Code" }),
    );
    await screen.findByRole("dialog", { name: "Repair Claude Code?" });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(complete).not.toHaveBeenCalled();
    expect((await base.getBootstrap()).onboardingComplete).toBe(true);
  });
  it("reconciles the sidebar after writer recovery without clearing other failures", async () => {
    globalThis.location.hash = "#/health";
    const client = createBrowserFixtureClient("health-degraded");
    render(
      <CommandProvider client={client}>
        <AnnouncementProvider>
          <App />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await screen.findByRole("heading", { name: "Health" });
    await openWarnings(userEvent.setup(), "Another Cutokyo window");
    await userEvent.click(
      screen.getByRole("button", { name: "Retry writer lock" }),
    );
    await waitFor(async () => {
      const current = await client.getHealth();
      expect(
        current.dimensions.find((entry) => entry.id === "writer_lock")?.state,
      ).toBe("healthy");
    });
    const trigger = screen.getByRole("button", { name: /^Warnings/ });
    await userEvent.click(trigger);
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: /^Warnings for/ }).textContent,
      ).not.toContain("Another Cutokyo window"),
    );
    const health = await client.getHealth();
    expect(
      health.dimensions.find((entry) => entry.id === "writer_lock")?.state,
    ).toBe("healthy");
    expect(health.dimensions.some((entry) => entry.state === "degraded")).toBe(
      true,
    );
    expect(screen.getByRole("heading", { name: "Health" })).toBeTruthy();
  });
});

import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setAppearancePreference } from "../appearance.js";
import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import type {
  CommandClient,
  DesktopSettings,
  ProxyStatus,
} from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { SettingsPage } from "./SettingsPage.js";

function renderSettings(client: CommandClient) {
  return render(
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <SettingsPage />
      </AnnouncementProvider>
    </CommandProvider>,
  );
}

function option(name: "Light" | "Dark" | "System") {
  return screen.getByRole("button", { name, pressed: true });
}

beforeEach(() => {
  setAppearancePreference("system");
});

afterEach(() => {
  setAppearancePreference("system");
});

describe("settings appearance preference", () => {
  it("selects an accessible segment immediately, saves only appearance, and keeps it on remount", async () => {
    const client = createBrowserFixtureClient("visual-keyboard");
    const patch = vi.fn(client.patchDesktopPreferences);
    let finishSave!: (settings: DesktopSettings) => void;
    const boundary: CommandClient = {
      ...client,
      patchDesktopPreferences: (change) => {
        void patch(change).then((settings) => finishSave(settings));
        return new Promise<DesktopSettings>((resolve) => {
          finishSave = resolve;
        });
      },
    };
    const user = userEvent.setup();
    const view = renderSettings(boundary);
    await screen.findByRole("heading", { name: "Settings" });
    await waitFor(() => expect(option("System")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Dark" }));
    expect(option("Dark")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(patch).toHaveBeenCalledWith({ appearance: "dark" });
    await screen.findByText("Appearance set to dark.");
    expect((await client.getSettings()).appearance).toBe("dark");
    view.unmount();
    renderSettings(client);
    await waitFor(() => expect(option("Dark")).toBeTruthy());
  });

  it("ignores an older read after saving Dark but accepts another window's newer Light", async () => {
    const client = createBrowserFixtureClient("visual-keyboard");
    const stale = await client.getSettings();
    let finishStaleRead!: (settings: DesktopSettings) => void;
    let reads = 0;
    renderSettings({
      ...client,
      getSettings: () => {
        reads += 1;
        if (reads === 2) {
          return new Promise((resolve) => {
            finishStaleRead = resolve;
          });
        }
        return client.getSettings();
      },
    });
    const user = userEvent.setup();
    await screen.findByRole("heading", { name: "Settings" });
    const search = screen.getByRole("switch", {
      name: /agent search/i,
    });
    await user.click(search);
    await waitFor(() => expect(reads).toBe(2));
    await user.click(screen.getByRole("button", { name: "Dark" }));
    await screen.findByText("Appearance set to dark.");
    await waitFor(() => expect(reads).toBe(3));
    expect((await client.getSettings()).appearance).toBe("dark");

    await act(async () => finishStaleRead(stale));
    expect(option("Dark")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("dark");

    await client.patchDesktopPreferences({ appearance: "light" });
    await user.click(search);
    await waitFor(() => expect(reads).toBe(4));
    await waitFor(() => expect(option("Light")).toBeTruthy());
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("rereads a concurrent-write winner instead of restoring the old Dark choice", async () => {
    const client = createBrowserFixtureClient("appearance-dark");
    let onDisk = await client.getSettings();
    const getSettings = vi.fn(async () => structuredClone(onDisk));
    const patchDesktopPreferences = vi.fn(async () => {
      onDisk = { ...onDisk, appearance: "system" };
      return Promise.reject(
        "Desktop settings changed in another window; reload before saving.",
      );
    });
    renderSettings({ ...client, getSettings, patchDesktopPreferences });
    const user = userEvent.setup();
    await waitFor(() => expect(option("Dark")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    await screen.findByRole("alert");
    await waitFor(() => expect(option("System")).toBeTruthy());
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("alert").textContent).toContain(
      "Saved appearance is system; your light choice was not saved.",
    );
    expect(getSettings.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(patchDesktopPreferences).toHaveBeenCalledWith({
      appearance: "light",
    });
    expect(onDisk.appearance).toBe("system");
  });

  it("keeps an unconfirmed conflict preview explicit until a retry can read the winner", async () => {
    const client = createBrowserFixtureClient("appearance-dark");
    const original = await client.getSettings();
    let reads = 0;
    const getSettings = vi.fn(async (): Promise<DesktopSettings> => {
      reads += 1;
      if (reads === 2) throw new Error("Temporary settings read failure");
      return { ...original, appearance: reads === 1 ? "dark" : "system" };
    });
    renderSettings({
      ...client,
      getSettings,
      patchDesktopPreferences: () =>
        Promise.reject(
          new Error(
            "Desktop settings changed in another window; reload before saving.",
          ),
        ),
    });
    const user = userEvent.setup();
    await waitFor(() => expect(option("Dark")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Light" }));
    await screen.findByText(/temporary preview, not a saved choice/i);
    expect(
      screen
        .getByRole("button", { name: "Light" })
        .getAttribute("aria-disabled"),
    ).toBe("true");
    await user.click(
      screen.getByRole("button", { name: "Retry reading saved appearance" }),
    );
    await waitFor(() => expect(option("System")).toBeTruthy());
    expect(reads).toBeGreaterThanOrEqual(3);
    expect(
      screen.queryByRole("button", { name: "Retry reading saved appearance" }),
    ).toBeNull();
  });

  it("reverts the selected state and theme if saving fails, with an announced error", async () => {
    const client = createBrowserFixtureClient("appearance-save-error");
    const user = userEvent.setup();
    renderSettings(client);
    await waitFor(() => expect(option("System")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    await screen.findByText(/Could not save appearance/);
    expect(option("System")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect((await client.getSettings()).appearance).toBe("system");
  });

  it("restores the previously saved override rather than System after a failed save", async () => {
    const client = createBrowserFixtureClient("appearance-dark");
    renderSettings({
      ...client,
      patchDesktopPreferences: () =>
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error("Disk is read-only")), 18),
        ),
    });
    const user = userEvent.setup();
    await waitFor(() => expect(option("Dark")).toBeTruthy());
    await user.click(screen.getByRole("button", { name: "Light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    await screen.findByText(/Disk is read-only/);
    expect(option("Dark")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

describe("proxy capture without guardrails", () => {
  it("locks mutation synchronously, blocks busy dismissal, and keeps failure inside the dialog", async () => {
    const client = createBrowserFixtureClient("proxy-capture-consent");
    let fail!: (error: Error) => void;
    const activate = vi.fn(
      () =>
        new Promise<ProxyStatus>((_, reject) => {
          fail = reject;
        }),
    );
    renderSettings({
      ...client,
      getProxyStatus: async () => ({
        ...(await client.getProxyStatus()),
        proxyStatus: "inactive",
      }),
      setProxyEnabled: activate,
    });
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Enable proxy" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Enable local proxy capture?",
    });
    await screen.findByText("Local redaction");
    const confirm = screen.getByRole("button", {
      name: "I understand — enable proxy",
    }) as HTMLButtonElement;
    act(() => {
      confirm.click();
      confirm.click();
    });
    expect(activate).toHaveBeenCalledTimes(1);
    expect(activate).toHaveBeenCalledWith(true, "proxy-consent:73A9");
    expect(
      (
        screen.getByRole("button", {
          name: "Close dialog",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Not now" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBe(dialog);
    await act(async () =>
      fail(new Error("Synthetic listener could not start")),
    );
    expect(within(dialog).getByRole("alert").textContent).toContain(
      "Synthetic listener could not start",
    );
    expect(
      (
        screen.getByRole("button", {
          name: "I understand — enable proxy",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(activate).toHaveBeenCalledTimes(1);
  });

  it("clears a persisted request without claiming an unavailable listener was stopped", async () => {
    const client = createBrowserFixtureClient("proxy-capture-consent");
    let requested = true;
    const activate = vi.fn(async (enabled: boolean, token: string | null) => {
      const status = await client.setProxyEnabled(enabled, token);
      requested = enabled;
      return status;
    });
    renderSettings({
      ...client,
      getProxyStatus: async () => ({
        ...(await client.getProxyStatus()),
        proxyEnabled: requested,
      }),
      setProxyEnabled: activate,
    });
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Clear proxy request" }),
    );
    expect(activate).toHaveBeenCalledWith(false, null);
    await screen.findByText(
      "Proxy capture request cleared; no listener was running.",
    );
    expect((await client.getProxyStatus()).proxyStatus).toBe("unavailable");
    expect((await client.getSettings()).proxy_enabled).toBe(false);
    expect(screen.queryByText("Proxy capture stopped.")).toBeNull();
  });

  it("hides proxy controls and guard switches while this build has no proxy", async () => {
    const client = createBrowserFixtureClient("proxy-capture-consent");
    const activate = vi.fn(client.setProxyEnabled);
    renderSettings({ ...client, setProxyEnabled: activate });
    await screen.findByRole("heading", { name: "Settings" });
    expect(screen.queryByRole("switch", { name: /guard/i })).toBeNull();
    expect(screen.queryByText(/Proxy capture/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Enable proxy" })).toBeNull();
    expect(activate).not.toHaveBeenCalled();
    expect((await client.getProxyStatus()).proxyStatus).toBe("unavailable");
  });
});

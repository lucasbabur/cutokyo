import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DesktopSettings } from "./contracts.js";
import { createBrowserFixtureClient } from "./fixtures/browserAdapter.js";

interface FakeMedia {
  matches: boolean;
  change(matches: boolean): void;
}

let media: FakeMedia;
let appearance: typeof import("./appearance.js");

beforeEach(async () => {
  vi.resetModules();
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  media = {
    matches: false,
    change(matches) {
      this.matches = matches;
      for (const listener of listeners) {
        listener({ matches } as MediaQueryListEvent);
      }
    },
  };
  vi.stubGlobal("matchMedia", (query: string) => ({
    get matches() {
      return media.matches;
    },
    media: query,
    addEventListener: (
      _type: string,
      listener: (event: MediaQueryListEvent) => void,
    ) => listeners.add(listener),
  }));
  document.head.innerHTML = '<meta name="theme-color" content="#f7f1e7">';
  appearance = await import("./appearance.js");
});

afterEach(() => {
  vi.useRealTimers();
  document.querySelector("#root")?.remove();
  vi.unstubAllGlobals();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.style.colorScheme = "";
  document.head.innerHTML = "";
});

function expectTheme(theme: "light" | "dark") {
  expect(document.documentElement.dataset.theme).toBe(theme);
  expect(document.documentElement.style.colorScheme).toBe(theme);
  expect(
    document.querySelector('meta[name="theme-color"]')?.getAttribute("content"),
  ).toBe(theme === "dark" ? "#111411" : "#f7f1e7");
}

describe("desktop appearance manager", () => {
  it("applies System before mount and follows live OS changes", () => {
    appearance.initializeAppearance();
    expectTheme("light");
    media.change(true);
    expectTheme("dark");
    media.change(false);
    expectTheme("light");
  });

  it("ignores OS changes under explicit overrides and follows again on System", () => {
    appearance.initializeAppearance();
    appearance.setAppearancePreference("dark");
    media.change(false);
    expectTheme("dark");
    appearance.setAppearancePreference("light");
    media.change(true);
    expectTheme("light");
    appearance.setAppearancePreference("system");
    expectTheme("dark");
    media.change(false);
    expectTheme("light");
  });

  it("loads the saved preference before mounting when available", async () => {
    const getSettings = vi.fn(
      createBrowserFixtureClient("appearance-dark").getSettings,
    );
    await appearance.loadAppearance({ getSettings });
    expect(getSettings).toHaveBeenCalledOnce();
    expectTheme("dark");
  });

  it("does not block startup on an unavailable settings command", async () => {
    await appearance.loadAppearance({
      getSettings: () => Promise.reject(new Error("Settings unavailable")),
    });
    expectTheme("light");
  });

  it("keeps the app unmounted past the timeout until a slow saved theme arrives", async () => {
    const saved =
      await createBrowserFixtureClient("appearance-dark").getSettings();
    vi.useFakeTimers();
    const root = document.createElement("div");
    root.id = "root";
    document.body.append(root);
    let finishRead!: (value: DesktopSettings) => void;
    let mounted = false;
    const pending = appearance.loadAppearance({
      getSettings: () =>
        new Promise((resolve) => {
          finishRead = resolve;
        }),
    });
    void pending.then(() => {
      mounted = true;
    });
    await vi.advanceTimersByTimeAsync(501);
    expect(mounted).toBe(false);
    expect(root.textContent).toContain("Waiting for your saved appearance");
    expectTheme("light");
    finishRead(saved);
    await pending;
    expect(mounted).toBe(true);
    expectTheme("dark");
    expect(root.textContent).toBe("");
  });

  it("offers an explicit device-theme fallback if the settings read hangs", async () => {
    vi.useFakeTimers();
    const root = document.createElement("div");
    root.id = "root";
    document.body.append(root);
    const pending = appearance.loadAppearance({
      getSettings: () => new Promise<DesktopSettings>(() => undefined),
    });
    await vi.advanceTimersByTimeAsync(500);
    const button = root.querySelector("button");
    expect(button?.textContent).toBe("Continue with device appearance");
    expect(root.textContent).toContain("Your saved choice will apply");
    button?.click();
    await pending;
    expect(root.textContent).toBe("");
    expectTheme("light");
  });

  it("ignores a late startup snapshot after a newer authoritative settings read", async () => {
    let finishRead!: (value: DesktopSettings) => void;
    const stale =
      await createBrowserFixtureClient("appearance-dark").getSettings();
    const pending = appearance.loadAppearance(
      {
        getSettings: () =>
          new Promise((resolve) => {
            finishRead = resolve;
          }),
      },
      1,
    );
    await pending;
    appearance.syncAppearancePreference("light");
    finishRead(stale);
    await Promise.resolve();
    expectTheme("light");
  });

  it("does not allow a late startup read to replace a user's choice", async () => {
    let finishRead!: (value: DesktopSettings) => void;
    const saved =
      await createBrowserFixtureClient("appearance-dark").getSettings();
    const pending = appearance.loadAppearance(
      {
        getSettings: () =>
          new Promise((resolve) => {
            finishRead = resolve;
          }),
      },
      1,
    );
    await pending;
    appearance.setAppearancePreference("light");
    finishRead(saved);
    await Promise.resolve();
    expectTheme("light");
  });
});

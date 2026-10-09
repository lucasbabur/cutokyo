import { beforeEach, describe, expect, it, vi } from "vitest";

import { isCutokyoAutostartEnabled, syncCutokyoAutostart } from "@/infrastructure/tauri/autostart";

const plugin = vi.hoisted(() => ({
  disable: vi.fn(async () => undefined),
  enable: vi.fn(async () => undefined),
  isEnabled: vi.fn(async () => false),
}));

vi.mock("@tauri-apps/plugin-autostart", () => plugin);

type TauriTestWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

beforeEach(() => {
  vi.clearAllMocks();
  plugin.isEnabled.mockResolvedValue(false);
  (window as TauriTestWindow).__TAURI_INTERNALS__ = {};
});

describe("Cutokyo autostart", () => {
  it("enables recovery startup only when it was disabled", async () => {
    await syncCutokyoAutostart(true);
    expect(plugin.enable).toHaveBeenCalledOnce();
    expect(plugin.disable).not.toHaveBeenCalled();

    plugin.isEnabled.mockResolvedValue(true);
    await syncCutokyoAutostart(true);
    expect(plugin.enable).toHaveBeenCalledOnce();
  });

  it("disables startup after safe deactivation", async () => {
    plugin.isEnabled.mockResolvedValue(true);
    await syncCutokyoAutostart(false);
    expect(plugin.disable).toHaveBeenCalledOnce();
  });

  it("reports the native startup state and is inert in a browser", async () => {
    plugin.isEnabled.mockResolvedValue(true);
    expect(await isCutokyoAutostartEnabled()).toBe(true);

    delete (window as TauriTestWindow).__TAURI_INTERNALS__;
    expect(await isCutokyoAutostartEnabled()).toBe(false);
    await syncCutokyoAutostart(true);
    expect(plugin.enable).not.toHaveBeenCalled();
  });
});

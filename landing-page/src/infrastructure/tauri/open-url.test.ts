import { beforeEach, describe, expect, it, vi } from "vitest";

import { openExternalUrl } from "./open-url";

const plugin = vi.hoisted(() => ({
  openUrl: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/plugin-opener", () => plugin);

type TauriTestWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

beforeEach(() => {
  vi.clearAllMocks();
  delete (window as TauriTestWindow).__TAURI_INTERNALS__;
});

describe("openExternalUrl", () => {
  it("opens normalized HTTPS URLs in a new browser context", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(window);

    await openExternalUrl("https://auth.example.test/callback?state=one two");

    expect(open).toHaveBeenCalledWith(
      "https://auth.example.test/callback?state=one%20two",
      "_blank",
      "noopener,noreferrer",
    );
    expect(plugin.openUrl).not.toHaveBeenCalled();
  });

  it("uses the native opener inside Tauri", async () => {
    (window as TauriTestWindow).__TAURI_INTERNALS__ = {};
    const open = vi.spyOn(window, "open");

    await openExternalUrl("http://localhost:49321/cutokyo/auth/start");

    expect(plugin.openUrl).toHaveBeenCalledWith("http://localhost:49321/cutokyo/auth/start");
    expect(open).not.toHaveBeenCalled();
  });

  it.each(["http://auth.example.test/start", "http://localhost.evil.test/start"])(
    "rejects plaintext HTTP to a non-loopback host: %s",
    async (url) => {
      const open = vi.spyOn(window, "open");

      await expect(openExternalUrl(url)).rejects.toThrow(
        "External HTTP URLs must use a loopback host.",
      );
      expect(plugin.openUrl).not.toHaveBeenCalled();
      expect(open).not.toHaveBeenCalled();
    },
  );

  it("does not navigate the webview when the native opener fails", async () => {
    (window as TauriTestWindow).__TAURI_INTERNALS__ = {};
    plugin.openUrl.mockRejectedValueOnce(new Error("opener unavailable"));
    const open = vi.spyOn(window, "open");

    await expect(openExternalUrl("https://auth.example.test/start")).rejects.toThrow(
      "opener unavailable",
    );

    expect(open).not.toHaveBeenCalled();
  });

  it.each(["javascript:alert(1)", "data:text/html,unsafe", "/relative-auth"])(
    "rejects an unsafe external URL: %s",
    async (url) => {
      const open = vi.spyOn(window, "open");

      await expect(openExternalUrl(url)).rejects.toThrow(
        url.startsWith("/") ? /invalid url/i : "External URLs must use HTTP or HTTPS.",
      );
      expect(plugin.openUrl).not.toHaveBeenCalled();
      expect(open).not.toHaveBeenCalled();
    },
  );
});

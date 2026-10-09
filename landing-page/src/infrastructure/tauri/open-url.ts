import "client-only";

type TauriWindow = Window & {
  __TAURI__?: unknown;
  __TAURI_INTERNALS__?: unknown;
};

const LOOPBACK_HOSTNAMES = new Set(["localhost", "127.0.0.1", "[::1]"]);

export async function openExternalUrl(url: string): Promise<void> {
  const parsedUrl = new URL(url);
  if (!new Set(["http:", "https:"]).has(parsedUrl.protocol)) {
    throw new Error("External URLs must use HTTP or HTTPS.");
  }
  if (parsedUrl.protocol === "http:" && !LOOPBACK_HOSTNAMES.has(parsedUrl.hostname)) {
    throw new Error("External HTTP URLs must use a loopback host.");
  }

  if (isTauriRuntime()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(parsedUrl.href);
    return;
  }

  const openedWindow = window.open(parsedUrl.href, "_blank", "noopener,noreferrer");
  if (!openedWindow) {
    window.location.assign(parsedUrl.href);
  }
}

function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  const tauriWindow = window as TauriWindow;
  return Boolean(tauriWindow.__TAURI_INTERNALS__ ?? tauriWindow.__TAURI__);
}

import "client-only";

type TauriWindow = Window & {
  __TAURI__?: unknown;
  __TAURI_INTERNALS__?: unknown;
};

export async function syncCutokyoAutostart(enabled: boolean): Promise<void> {
  if (!isTauriRuntime()) return;

  const { disable, enable, isEnabled } = await import("@tauri-apps/plugin-autostart");
  const currentlyEnabled = await isEnabled();

  if (enabled && !currentlyEnabled) {
    await enable();
    return;
  }

  if (!enabled && currentlyEnabled) {
    await disable();
  }
}

export async function isCutokyoAutostartEnabled(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  const { isEnabled } = await import("@tauri-apps/plugin-autostart");
  return isEnabled();
}

function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  const tauriWindow = window as TauriWindow;
  return Boolean(tauriWindow.__TAURI_INTERNALS__ ?? tauriWindow.__TAURI__);
}

import { invoke } from "@tauri-apps/api/core";
import * as event from "@tauri-apps/api/event";

interface NativeE2eGlobal {
  __TAURI__?: {
    core: { invoke: typeof invoke };
    event: typeof event;
  };
}

export async function installNativeE2eBridge(): Promise<void> {
  if (import.meta.env.MODE !== "native-e2e") return;

  // Keep production `withGlobalTauri` disabled. The official WDIO frontend
  // plugin receives only the imported core/event modules in this test bundle.
  const nativeGlobal = globalThis as typeof globalThis & NativeE2eGlobal;
  Object.defineProperty(nativeGlobal, "__TAURI__", {
    configurable: true,
    value: {
      core: { invoke },
      event,
    },
  });

  const { init } = await import("@wdio/tauri-plugin");
  await init();
}

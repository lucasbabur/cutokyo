import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import { CommandProvider } from "./commands/context.js";
import { createTauriCommandClient } from "./commands/tauri.js";
import { AnnouncementProvider } from "./components/Announcer.js";
import type { CommandClient } from "./contracts.js";
import "./styles.css";

async function resolveCommandClient(): Promise<CommandClient> {
  if (import.meta.env.MODE === "native-e2e") {
    const { installNativeE2eBridge } =
      await import("./testing/nativeE2eBridge.js");
    await installNativeE2eBridge();
  }

  if (
    import.meta.env.DEV &&
    import.meta.env.VITE_CUTOKYO_BROWSER_FIXTURES === "1"
  ) {
    const { createBrowserFixtureClient } =
      await import("./fixtures/browserAdapter.js");
    const selector = new URLSearchParams(globalThis.location.search).get(
      "jev_case",
    );
    const client = createBrowserFixtureClient(selector);
    Object.defineProperty(globalThis, "__CUTOKYO_FIXTURE_AUDIT__", {
      configurable: true,
      value: () => client.fixtureAudit(),
    });
    return client;
  }
  return createTauriCommandClient();
}

const container = document.querySelector<HTMLDivElement>("#root");
if (container === null) throw new Error("Cutokyo root element is missing.");

void resolveCommandClient()
  .then((client) => {
    createRoot(container).render(
      <StrictMode>
        <CommandProvider client={client}>
          <AnnouncementProvider>
            <App />
          </AnnouncementProvider>
        </CommandProvider>
      </StrictMode>,
    );
  })
  .catch((reason: unknown) => {
    const message = reason instanceof Error ? reason.message : String(reason);
    container.innerHTML = "";
    const alert = document.createElement("div");
    alert.className = "fatal-startup";
    alert.setAttribute("role", "alert");
    alert.textContent = `Cutokyo could not initialize its application command boundary: ${message}`;
    container.append(alert);
  });

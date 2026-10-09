import { useSyncExternalStore } from "react";

import type { CommandClient, DesktopSettings } from "./contracts.js";

export type Appearance = DesktopSettings["appearance"];
type Theme = "light" | "dark";

const preferenceQuery = "(prefers-color-scheme: dark)";
const listeners = new Set<() => void>();
let preference: Appearance = "system";
let media: MediaQueryList | null = null;
let choiceRevision = 0;
let authoritativeReadRevision = 0;

/** Capture before a settings read so its result cannot undo a newer choice. */
export function appearanceChoiceRevision(): number {
  return choiceRevision;
}

function applyTheme() {
  const theme: Theme =
    preference === "system" ? (media?.matches ? "dark" : "light") : preference;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", theme === "dark" ? "#111411" : "#f7f1e7");
}

/** Set an OS-derived theme before React renders, then follow OS changes only in System. */
export function initializeAppearance() {
  if (media === null) {
    media = window.matchMedia(preferenceQuery);
    media.addEventListener("change", () => {
      if (preference === "system") applyTheme();
    });
  }
  applyTheme();
}

function updatePreference(next: Appearance) {
  initializeAppearance();
  if (preference === next) return;
  preference = next;
  applyTheme();
  for (const listener of listeners) listener();
}

/** A user's choice wins over a startup settings read that finishes late. */
export function setAppearancePreference(next: Appearance) {
  choiceRevision += 1;
  updatePreference(next);
}

/** Reflect an authoritative settings response without marking it as a user edit. */
export function syncAppearancePreference(next: Appearance) {
  authoritativeReadRevision += 1;
  updatePreference(next);
}

export function useAppearancePreference(): Appearance {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => preference,
  );
}

/** Mount only after the saved preference is known (or the user accepts a device-theme fallback). */
export async function loadAppearance(
  client: Pick<CommandClient, "getSettings">,
  timeoutMs = 500,
): Promise<void> {
  initializeAppearance();
  const startedAtRevision = choiceRevision;
  const startedAtReadRevision = authoritativeReadRevision;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  let prompt: HTMLElement | undefined;
  const read = Promise.resolve()
    .then(() => client.getSettings())
    .then((settings) => {
      if (
        startedAtRevision === choiceRevision &&
        startedAtReadRevision === authoritativeReadRevision
      ) {
        syncAppearancePreference(settings.appearance);
      }
    })
    .catch(() => {
      // A failed settings command must not block the rest of the application.
    });
  const fallback = new Promise<void>((resolve) => {
    timeout = setTimeout(() => {
      const root = document.querySelector("#root");
      if (root === null) {
        resolve();
        return;
      }
      prompt = document.createElement("div");
      prompt.className = "appearance-startup";
      prompt.setAttribute("role", "status");
      const message = document.createElement("p");
      message.textContent = "Waiting for your saved appearance…";
      const continueButton = document.createElement("button");
      continueButton.type = "button";
      continueButton.textContent = "Continue with device appearance";
      continueButton.setAttribute(
        "aria-label",
        "Continue with device appearance while saved settings load",
      );
      continueButton.addEventListener("click", () => resolve(), { once: true });
      const note = document.createElement("p");
      note.textContent =
        "Your saved choice will apply if settings become available.";
      prompt.append(message, continueButton, note);
      root.append(prompt);
    }, timeoutMs);
  });
  try {
    await Promise.race([read, fallback]);
  } finally {
    clearTimeout(timeout);
    prompt?.remove();
  }
}

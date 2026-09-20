import type { SettingsPatch } from "./generated/settings.js";

/** Applies the public omission-preserving patch semantics used by desktop forms. */
export function applySettingsPatch<T extends Readonly<Record<string, unknown>>>(
  current: T,
  patch: SettingsPatch,
): T & SettingsPatch {
  return { ...current, ...patch };
}

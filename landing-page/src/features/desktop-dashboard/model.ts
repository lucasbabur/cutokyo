import type {
  AttributedCategories as LocalAttributedCategories,
  DesktopSnapshot as LocalDesktopSnapshot,
  FeatureSettings as LocalFeatureSettings,
  LocalSession as LocalSessionModel,
  OperationEvent as LocalOperationEvent,
} from "@/infrastructure/cutokyo-local/dashboard";
import {
  CATEGORY_LABELS,
  providerAlignedContext,
  useLocalDesktopDashboard,
} from "@/infrastructure/cutokyo-local/dashboard";

export { CATEGORY_LABELS, providerAlignedContext };
export const useDesktopDashboard = useLocalDesktopDashboard;
export type AttributedCategories = LocalAttributedCategories;
export type DesktopSnapshot = LocalDesktopSnapshot;
export type DesktopView = "context" | "overview" | "plugins" | "sessions" | "settings";
export type FeatureSettings = LocalFeatureSettings;
export type LocalSession = LocalSessionModel;
export type OperationEvent = LocalOperationEvent;

export function initialDesktopView(): DesktopView {
  if (typeof window === "undefined") return "overview";
  const requested = new URLSearchParams(window.location.search).get("view");
  return requested && ["context", "overview", "plugins", "sessions", "settings"].includes(requested)
    ? (requested as DesktopView)
    : "overview";
}

export function serverDesktopView(): DesktopView {
  return "overview";
}

const DESKTOP_VIEW_EVENT = "cutokyo:desktop-view";

export function navigateDesktopView(view: DesktopView): void {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  if (view === "overview") url.searchParams.delete("view");
  else url.searchParams.set("view", view);
  window.history.pushState(null, "", `${url.pathname}${url.search}${url.hash}`);
  window.dispatchEvent(new Event(DESKTOP_VIEW_EVENT));
}

export function subscribeDesktopView(listener: () => void): () => void {
  if (typeof window === "undefined") return () => undefined;
  window.addEventListener("popstate", listener);
  window.addEventListener(DESKTOP_VIEW_EVENT, listener);
  return () => {
    window.removeEventListener("popstate", listener);
    window.removeEventListener(DESKTOP_VIEW_EVENT, listener);
  };
}

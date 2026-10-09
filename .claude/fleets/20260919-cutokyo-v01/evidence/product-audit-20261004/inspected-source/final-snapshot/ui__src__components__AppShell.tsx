import {
  Activity,
  Bot,
  Boxes,
  Database,
  Gauge,
  History,
  Settings,
  Stethoscope,
} from "lucide-react";
import { type PropsWithChildren, useEffect, useRef } from "react";

import type { BootstrapResponse, RoutePath } from "../contracts.js";
import { routeHref } from "../router.js";
import { StatusPill } from "./Primitives.js";

const NAV_ITEMS: readonly {
  readonly path: RoutePath;
  readonly label: string;
  readonly shortLabel: string;
  readonly icon: typeof Gauge;
}[] = [
  {
    path: "/dashboard",
    label: "Overview",
    shortLabel: "Overview",
    icon: Gauge,
  },
  {
    path: "/sessions",
    label: "Sessions",
    shortLabel: "Sessions",
    icon: History,
  },
  {
    path: "/analysis",
    label: "AI analysis",
    shortLabel: "Analysis",
    icon: Bot,
  },
  {
    path: "/inventory",
    label: "Agent inventory",
    shortLabel: "Inventory",
    icon: Boxes,
  },
  {
    path: "/health",
    label: "System health",
    shortLabel: "Health",
    icon: Stethoscope,
  },
  { path: "/data", label: "Data controls", shortLabel: "Data", icon: Database },
  {
    path: "/settings",
    label: "Settings",
    shortLabel: "Settings",
    icon: Settings,
  },
];

export function AppShell({
  bootstrap,
  currentPath,
  children,
}: PropsWithChildren<{
  readonly bootstrap: BootstrapResponse;
  readonly currentPath: RoutePath;
}>) {
  const renderedPath = useRef(currentPath);
  useEffect(() => {
    // The first load keeps natural tab order; only route changes move focus.
    if (renderedPath.current === currentPath) return;
    renderedPath.current = currentPath;
    // Pages render a loading state before their heading, so wait for it.
    const focusHeading = () => {
      const heading = document.querySelector<HTMLElement>(
        "[data-page-heading]",
      );
      heading?.focus();
      return heading !== null;
    };
    const frame = globalThis.requestAnimationFrame(() => {
      if (focusHeading()) return;
      observer.observe(document.body, { childList: true, subtree: true });
    });
    const observer = new MutationObserver(() => {
      if (focusHeading()) observer.disconnect();
    });
    return () => {
      globalThis.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [currentPath]);

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>
      <aside className="sidebar" aria-label="Application">
        <div className="brand">
          <div className="brand__mark" aria-hidden="true">
            <Activity />
          </div>
          <div>
            <strong>Cutokyo</strong>
            <span>Local observability</span>
          </div>
        </div>

        {bootstrap.onboardingComplete ? (
          <nav className="primary-nav" aria-label="Primary navigation">
            {NAV_ITEMS.map((item) => {
              const Icon = item.icon;
              const active = item.path === currentPath;
              return (
                <a
                  key={item.path}
                  href={routeHref(item.path)}
                  className={
                    active ? "primary-nav__item is-active" : "primary-nav__item"
                  }
                  aria-current={active ? "page" : undefined}
                  title={item.label}
                >
                  <Icon aria-hidden="true" />
                  <span className="primary-nav__label">{item.shortLabel}</span>
                </a>
              );
            })}
          </nav>
        ) : (
          <div className="sidebar__setup">
            <span className="sidebar__step">First run</span>
            <strong>Choose what Cutokyo may observe.</strong>
            <p>No proxy or model-provider request is enabled by default.</p>
          </div>
        )}

        <div className="sidebar__footer">
          <div className="connection-row">
            <span
              className={
                bootstrap.writerMode === "owner"
                  ? "connection-dot"
                  : "connection-dot is-warning"
              }
              aria-hidden="true"
            />
            <span>
              {bootstrap.writerMode === "owner"
                ? "Local writer"
                : "Read-only view"}
            </span>
          </div>
          <StatusPill state={bootstrap.localOnly ? "enabled" : "active"}>
            {bootstrap.localOnly ? "Local only" : "Proxy active"}
          </StatusPill>
          <span className="sidebar__version">v{bootstrap.appVersion}</span>
        </div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <div className="topbar__scope">
            <span>Workspace</span>
            <strong>All local projects</strong>
          </div>
          <div
            className="topbar__status"
            role="group"
            aria-label="Capture and egress status"
          >
            {bootstrap.proxyActive ? (
              <span className="live-indicator">
                <span aria-hidden="true" /> Proxy capture live
              </span>
            ) : (
              <span className="quiet-indicator">Proxy off</span>
            )}
            <span className="divider" aria-hidden="true" />
            <span className="quiet-indicator">
              AI egress{" "}
              {bootstrap.analysisEgressEnabled
                ? "enabled"
                : "on confirmation only"}
            </span>
          </div>
        </header>
        <main id="main-content" className="main-content" tabIndex={-1}>
          {bootstrap.startupNotice === null ? null : (
            <div className="startup-notice" role="status">
              {bootstrap.startupNotice}
            </div>
          )}
          {children}
        </main>
      </div>
    </div>
  );
}

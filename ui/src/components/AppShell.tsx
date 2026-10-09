import {
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
import { BrandMark } from "./BrandMark.js";
import {
  NoticesProvider,
  type ShellWarning,
  WarningsPanel,
} from "./Notices.js";

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
    path: "/inventory",
    label: "Agent tools",
    shortLabel: "Agent tools",
    icon: Boxes,
  },
  {
    path: "/health",
    label: "Health",
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
  onRecheckStatus,
  recheckingStatus = false,
  children,
}: PropsWithChildren<{
  readonly bootstrap: BootstrapResponse;
  readonly currentPath: RoutePath;
  readonly onRecheckStatus?: () => void;
  readonly recheckingStatus?: boolean;
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
    <NoticesProvider>
      <div className="app-shell">
        <a
          className="skip-link"
          href="#main-content"
          onClick={(event) => {
            event.preventDefault();
            document.getElementById("main-content")?.focus();
          }}
        >
          Skip to content
        </a>
        <aside className="sidebar" aria-label="Application">
          <div className="brand">
            <div className="brand__mark" aria-hidden="true">
              <BrandMark />
            </div>
            <div>
              <strong>Cutokyo</strong>
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
                      active
                        ? "primary-nav__item is-active"
                        : "primary-nav__item"
                    }
                    aria-current={active ? "page" : undefined}
                    title={item.label}
                  >
                    <Icon aria-hidden="true" />
                    <span className="primary-nav__label">
                      {item.shortLabel}
                    </span>
                  </a>
                );
              })}
            </nav>
          ) : (
            <div className="sidebar__spacer" />
          )}

          <div className="sidebar__footer">
            <WarningsPanel
              pageLabel={
                NAV_ITEMS.find((item) => item.path === currentPath)?.label ??
                "Setup"
              }
              extra={shellWarnings(bootstrap)}
              onRecheck={onRecheckStatus}
              rechecking={recheckingStatus}
            />
            <span className="sidebar__version">v{bootstrap.appVersion}</span>
          </div>
        </aside>

        <div className="workspace">
          <main id="main-content" className="main-content" tabIndex={-1}>
            {children}
          </main>
        </div>
      </div>
    </NoticesProvider>
  );
}

function shellWarnings(bootstrap: BootstrapResponse): readonly ShellWarning[] {
  const warnings: ShellWarning[] = [];
  (bootstrap.startupNotice ?? "")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .forEach((line, index) => {
      warnings.push({
        id: `startup-${index}`,
        tone: "partial",
        title: "Startup check",
        detail: line,
        source: "Startup",
      });
    });
  if (bootstrap.writerMode === "read_only") {
    warnings.push({
      id: "writer-read-only",
      tone: "partial",
      title: "Writer held elsewhere",
      source: "Startup",
      detail:
        "Another Cutokyo window is editing history. This one is read-only.",
    });
  } else if (bootstrap.writerMode === "unavailable") {
    warnings.push({
      id: "writer-unavailable",
      tone: "degraded",
      title: "Local history unavailable",
      source: "Startup",
      detail: "History could not be opened. See Health.",
    });
  }
  if (bootstrap.proxyActive) {
    warnings.push({
      id: "proxy-active",
      tone: "info",
      title: "Proxy capture is on",
      source: "Startup",
      detail: "Requests pass through the local proxy.",
    });
  }
  return warnings;
}

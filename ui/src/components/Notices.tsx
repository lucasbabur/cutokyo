import {
  AlertTriangle,
  BellRing,
  Check,
  Info,
  RefreshCw,
  X,
} from "lucide-react";
import {
  createContext,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";

import type { RouteMeta } from "../contracts.js";

interface Registration {
  readonly freshness: RouteMeta["freshness"];
  readonly notices: readonly string[];
}

interface NoticeRegistry {
  readonly register: (id: string, value: Registration | null) => void;
  readonly entries: readonly Registration[];
}

const NoticeContext = createContext<NoticeRegistry>({
  register: () => undefined,
  entries: [],
});

/** Collects each mounted route's data notices so the shell can show one badge. */
export function NoticesProvider({ children }: PropsWithChildren) {
  const [store, setStore] = useState<ReadonlyMap<string, Registration>>(
    new Map(),
  );
  const register = useCallback((id: string, value: Registration | null) => {
    setStore((current) => {
      const next = new Map(current);
      if (value === null) next.delete(id);
      else next.set(id, value);
      return next;
    });
  }, []);
  const entries = useMemo(() => [...store.values()], [store]);
  const registry = useMemo(() => ({ register, entries }), [register, entries]);
  return (
    <NoticeContext.Provider value={registry}>{children}</NoticeContext.Provider>
  );
}

/**
 * Registers a route's freshness and notices with the shell warnings panel.
 * Renders nothing: blocking failures use ErrorState and stay in the page.
 */
export function RouteNotice({ meta }: { readonly meta: RouteMeta }) {
  const id = useId();
  const { register } = useContext(NoticeContext);
  const key = JSON.stringify([meta.freshness, meta.notices]);
  useEffect(() => {
    const [freshness, notices] = JSON.parse(key) as [
      RouteMeta["freshness"],
      string[],
    ];
    register(
      id,
      freshness === "complete" && notices.length === 0
        ? null
        : { freshness, notices },
    );
    return () => register(id, null);
  }, [id, key, register]);
  return null;
}

export interface ShellWarning {
  readonly id: string;
  readonly tone: "degraded" | "partial" | "info";
  readonly title: string;
  readonly detail: string;
  /** Where the warning came from: Startup, or the page that reported it. */
  readonly source?: string;
}

function useRouteWarnings(): readonly ShellWarning[] {
  const { entries } = useContext(NoticeContext);
  return useMemo(
    () =>
      entries.flatMap((entry, entryIndex) => {
        const tone = entry.freshness === "complete" ? "info" : entry.freshness;
        if (entry.notices.length === 0) {
          return [
            {
              id: `fresh-${entryIndex}`,
              tone,
              title: `${entry.freshness === "degraded" ? "Degraded" : "Partial"} data`,
              detail:
                entry.freshness === "degraded"
                  ? "Some sources could not be read. Totals on this page may be incomplete."
                  : "Some sources are incomplete. Unknown values are shown as unknown, not zero.",
            } satisfies ShellWarning,
          ];
        }
        return entry.notices.map(
          (detail, index) =>
            ({
              id: `note-${entryIndex}-${index}`,
              tone,
              title:
                tone === "degraded"
                  ? "Degraded data"
                  : tone === "partial"
                    ? "Partial data"
                    : "Note",
              detail,
            }) satisfies ShellWarning,
        );
      }),
    [entries],
  );
}

export function WarningsPanel({
  pageLabel,
  extra,
  onRecheck,
  rechecking = false,
}: {
  readonly pageLabel: string;
  readonly extra: readonly ShellWarning[];
  readonly onRecheck?: (() => void) | undefined;
  readonly rechecking?: boolean;
}) {
  const routeWarnings = useRouteWarnings();
  const warnings = [
    ...extra,
    ...routeWarnings.map((entry) => ({ ...entry, source: pageLabel })),
  ];
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const actionable = warnings.some((entry) => entry.tone === "degraded");

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOpen(false);
      button.current?.focus();
    };
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (
        target !== null &&
        !panel.current?.contains(target) &&
        !button.current?.contains(target)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [open]);

  return (
    <div className="warnings">
      <button
        ref={button}
        type="button"
        className={`warnings__trigger${warnings.length > 0 ? " has-warnings" : ""}${actionable ? " is-actionable" : ""}`}
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((value) => !value)}
      >
        <BellRing aria-hidden="true" />
        <span>Warnings</span>
        {warnings.length === 0 ? null : (
          <span
            className="warnings__count"
            aria-label={`${warnings.length} active`}
          >
            {warnings.length}
          </span>
        )}
      </button>
      <span className="sr-only" role="status">
        {warnings.length === 0
          ? "No active warnings"
          : `${warnings.length} active ${warnings.length === 1 ? "warning" : "warnings"}`}
      </span>
      {open ? (
        <div
          ref={panel}
          id={panelId}
          className="warnings__panel"
          role="region"
          aria-label={`Warnings for ${pageLabel}`}
        >
          <header>
            <strong>Warnings</strong>
            <span className="warnings__header-actions">
              {onRecheck === undefined ? null : (
                <button
                  type="button"
                  className="warnings__icon-button"
                  aria-label="Recheck local status"
                  disabled={rechecking}
                  onClick={onRecheck}
                >
                  <RefreshCw aria-hidden="true" /> Recheck
                </button>
              )}
              <button
                type="button"
                className="warnings__icon-button"
                aria-label="Close warnings"
                onClick={() => {
                  setOpen(false);
                  button.current?.focus();
                }}
              >
                <X aria-hidden="true" />
              </button>
            </span>
          </header>
          {warnings.length === 0 ? (
            <p className="warnings__empty">
              <Check aria-hidden="true" /> No warnings
            </p>
          ) : (
            <ul>
              {warnings.map((warning) => (
                <li key={warning.id} className={`is-${warning.tone}`}>
                  {warning.tone === "info" ? (
                    <Info aria-hidden="true" />
                  ) : (
                    <AlertTriangle aria-hidden="true" />
                  )}
                  <div>
                    <div className="warnings__item-head">
                      <strong>{warning.title}</strong>
                      {warning.source === undefined ? null : (
                        <span className="warnings__source">
                          {warning.source}
                        </span>
                      )}
                    </div>
                    <p>{warning.detail}</p>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}

import {
  AlertTriangle,
  Check,
  CircleHelp,
  Database,
  Info,
  LoaderCircle,
  RefreshCw,
  X,
} from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type PropsWithChildren,
  type ReactNode,
  useEffect,
  useId,
  useRef,
} from "react";

import type {
  Confidence,
  Coverage,
  HealthState,
  Provenance,
  RouteMeta,
} from "../contracts.js";

export function Button({
  variant = "secondary",
  size = "regular",
  className = "",
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  readonly variant?: "primary" | "secondary" | "quiet" | "danger";
  readonly size?: "regular" | "small";
}) {
  return (
    <button
      type="button"
      className={`button button--${variant} button--${size} ${className}`.trim()}
      {...props}
    >
      {children}
    </button>
  );
}

export function StatusPill({
  state,
  children,
}: PropsWithChildren<{
  readonly state:
    | HealthState
    | Coverage["state"]
    | "enabled"
    | "disabled"
    | "active"
    | "inactive"
    | "observed"
    | "estimated"
    | "user_declared"
    | "conflicting"
    | "blocked"
    | "inspected"
    | "unavailable"
    | "unknown";
}>) {
  const normalized = state.replaceAll("_", "-");
  return (
    <span className={`status-pill status-pill--${normalized}`}>
      <span className="status-pill__icon" aria-hidden="true">
        {state === "healthy" ||
        state === "complete" ||
        state === "enabled" ||
        state === "observed"
          ? "✓"
          : state === "degraded" ||
              state === "partial" ||
              state === "conflicting"
            ? "!"
            : "·"}
      </span>
      {children}
    </span>
  );
}

export function CoverageBadge({ coverage }: { readonly coverage: Coverage }) {
  return (
    <StatusPill state={coverage.state}>
      {coverage.state.replaceAll("_", " ")}
    </StatusPill>
  );
}

export function ConfidenceBadge({
  confidence,
}: {
  readonly confidence: Confidence;
}) {
  return (
    <StatusPill state={confidence}>
      {confidence.replaceAll("_", " ")}
    </StatusPill>
  );
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  readonly eyebrow: string;
  readonly title: string;
  readonly description: string;
  readonly actions?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div>
        <p className="eyebrow">{eyebrow}</p>
        <h1 tabIndex={-1} data-page-heading>
          {title}
        </h1>
        <p className="page-header__description">{description}</p>
      </div>
      {actions === undefined ? null : (
        <div className="page-header__actions">{actions}</div>
      )}
    </header>
  );
}

export function RouteNotice({ meta }: { readonly meta: RouteMeta }) {
  if (meta.notices.length === 0 && meta.freshness === "complete") return null;
  return (
    <aside
      className={`route-notice route-notice--${meta.freshness}`}
      aria-label={`${meta.freshness} data notice`}
    >
      {meta.freshness === "degraded" ? (
        <AlertTriangle aria-hidden="true" />
      ) : (
        <Info aria-hidden="true" />
      )}
      <div>
        <strong>
          {meta.freshness === "complete"
            ? "Current data"
            : `${meta.freshness} data`}
        </strong>
        {meta.notices.map((notice) => (
          <p key={notice}>{notice}</p>
        ))}
      </div>
    </aside>
  );
}

export function LoadingState({ label }: { readonly label: string }) {
  return (
    <div className="route-state" role="status" aria-live="polite">
      <LoaderCircle className="spin" aria-hidden="true" />
      <div>
        <strong>{label}</strong>
        <p>
          Reading the local application service. No network request is made.
        </p>
      </div>
    </div>
  );
}

export function ErrorState({
  title,
  error,
  onRetry,
}: {
  readonly title: string;
  readonly error: Error;
  readonly onRetry: () => void;
}) {
  return (
    <div className="route-state route-state--error" role="alert">
      <AlertTriangle aria-hidden="true" />
      <div>
        <strong>{title}</strong>
        <p>{error.message}</p>
        <Button size="small" onClick={onRetry}>
          <RefreshCw aria-hidden="true" /> Retry
        </Button>
      </div>
    </div>
  );
}

export function EmptyState({
  title,
  description,
  action,
  compact = false,
}: {
  readonly title: string;
  readonly description: string;
  readonly action?: ReactNode;
  readonly compact?: boolean;
}) {
  return (
    <div className={`empty-state ${compact ? "empty-state--compact" : ""}`}>
      <Database aria-hidden="true" />
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
    </div>
  );
}

export function ProvenanceDetails({
  provenance,
  label = "View provenance",
}: {
  readonly provenance: Provenance;
  readonly label?: string;
}) {
  return (
    <details className="provenance">
      <summary>{label}</summary>
      <dl>
        <div>
          <dt>Winning source</dt>
          <dd>
            {provenance.channel.replaceAll("_", " ")} · tier{" "}
            {provenance.sourceTier}
          </dd>
        </div>
        <div>
          <dt>Confidence</dt>
          <dd>
            <ConfidenceBadge confidence={provenance.confidence} />
          </dd>
        </div>
        <div>
          <dt>Captured</dt>
          <dd>{formatDateTime(provenance.capturedAt)}</dd>
        </div>
        <div>
          <dt>Parser</dt>
          <dd>{provenance.parserVersion}</dd>
        </div>
        <div>
          <dt>Coverage</dt>
          <dd>{provenance.coverage.scope}</dd>
        </div>
        <div>
          <dt>Evidence</dt>
          <dd>{provenance.observationIds.join(", ")}</dd>
        </div>
      </dl>
      {provenance.coverage.gaps.length === 0 ? null : (
        <ul>
          {provenance.coverage.gaps.map((gap) => (
            <li key={gap}>{gap}</li>
          ))}
        </ul>
      )}
    </details>
  );
}

export function DefinitionList({
  rows,
}: {
  readonly rows: readonly {
    readonly term: string;
    readonly value: ReactNode;
  }[];
}) {
  return (
    <dl className="definition-list">
      {rows.map((row) => (
        <div key={row.term}>
          <dt>{row.term}</dt>
          <dd>{row.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Modal({
  title,
  description,
  children,
  footer,
  onClose,
  closeLabel = "Close dialog",
  size = "regular",
}: PropsWithChildren<{
  readonly title: string;
  readonly description?: string;
  readonly footer?: ReactNode;
  readonly onClose: () => void;
  readonly closeLabel?: string;
  readonly size?: "regular" | "wide";
}>) {
  const titleId = useId();
  const descriptionId = useId();
  const backdropRef = useRef<HTMLDivElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const root = modalRef.current;
    const inerted: { element: HTMLElement; previous: boolean }[] = [];
    let branch: HTMLElement | null = backdropRef.current;
    while (branch !== null && branch.parentElement !== null) {
      const parent: HTMLElement = branch.parentElement;
      for (const sibling of parent.children) {
        if (sibling instanceof HTMLElement && sibling !== branch) {
          inerted.push({ element: sibling, previous: sibling.inert });
          sibling.inert = true;
        }
      }
      branch = parent;
      if (parent === document.body) break;
    }
    root
      ?.querySelector<HTMLElement>("button, input, select, textarea, [href]")
      ?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab" || root === null) return;
      const focusable = [
        ...root.querySelectorAll<HTMLElement>(
          "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href]",
        ),
      ];
      const first = focusable[0];
      const last = focusable.at(-1);
      if (first === undefined || last === undefined) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      for (const { element, previous } of inerted) element.inert = previous;
      previousFocus?.focus();
    };
  }, []);

  return (
    <div ref={backdropRef} className="modal-backdrop" role="presentation">
      <div
        ref={modalRef}
        className={`modal modal--${size}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description === undefined ? undefined : descriptionId}
      >
        <header className="modal__header">
          <div>
            <p className="eyebrow">Confirmation</p>
            <h2 id={titleId}>{title}</h2>
            {description === undefined ? null : (
              <p id={descriptionId}>{description}</p>
            )}
          </div>
          <Button
            variant="quiet"
            size="small"
            onClick={onClose}
            aria-label={closeLabel}
          >
            <X aria-hidden="true" />
          </Button>
        </header>
        <div
          className="modal__body"
          role="region"
          tabIndex={0}
          aria-label="Dialog content"
        >
          {children}
        </div>
        {footer === undefined ? null : (
          <footer className="modal__footer">{footer}</footer>
        )}
      </div>
    </div>
  );
}

export function Disclosure({ children }: PropsWithChildren) {
  return (
    <div className="disclosure">
      <CircleHelp aria-hidden="true" />
      <p>{children}</p>
    </div>
  );
}

export function SuccessMessage({ children }: PropsWithChildren) {
  return (
    <div className="success-message" role="status">
      <Check aria-hidden="true" />
      <p>{children}</p>
    </div>
  );
}

export function formatDateTime(value: string | null): string {
  if (value === null) return "Unknown";
  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

export function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB`;
}

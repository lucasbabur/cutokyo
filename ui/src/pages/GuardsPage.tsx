import {
  EyeOff,
  LockKeyhole,
  Radio,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { ProxyPreview } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  RouteNotice,
  StatusPill,
} from "../components/Primitives.js";

export function GuardsPage() {
  const commands = useCommands();
  const resource = useCommandResource(() => commands.getGuards(), "guards");
  const [preview, setPreview] = useState<ProxyPreview | "loading" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const announce = useAnnounce();

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Reading guard and capture coverage" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Guard coverage is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data;
  if (data === null) return null;

  const beginProxy = async () => {
    setPreview("loading");
    setError(null);
    try {
      setPreview(await commands.previewProxy());
    } catch (reason) {
      setPreview(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const enableProxy = async () => {
    if (preview === null || preview === "loading") return;
    setBusy(true);
    try {
      await commands.setProxyEnabled(true, preview.consentToken);
      setPreview(null);
      announce("Proxy capture enabled after explicit consent.");
      resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  const disableProxy = async () => {
    setBusy(true);
    try {
      await commands.setProxyEnabled(false, null);
      announce("Proxy capture stopped.");
      resource.reload();
    } finally {
      setBusy(false);
    }
  };
  const toggleGuard = async () => {
    setBusy(true);
    try {
      await commands.setOutgoingGuardEnabled(!data.outgoingGuardEnabled);
      announce(
        `Outgoing guard ${data.outgoingGuardEnabled ? "disabled" : "enabled"}.`,
      );
      resource.reload();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <PageHeader
        eyebrow="Inspectability is not protection"
        title="Guards & capture"
        description="See exactly which surfaces are redacted, inspected, blocked, disabled, or unavailable. Provider traffic is never described as protected unless a supported guard inspects it."
        actions={
          data.proxyStatus === "active" ? (
            <span className="live-indicator">
              <span aria-hidden="true" /> Proxy capture live
            </span>
          ) : (
            <StatusPill state="inactive">Proxy inactive</StatusPill>
          )
        }
      />
      <RouteNotice meta={data.meta} />
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <section className="guard-controls" aria-label="Guard controls">
        <article>
          <div className="guard-controls__icon">
            <ShieldAlert aria-hidden="true" />
          </div>
          <div>
            <h2>Outgoing secret guard</h2>
            <p>
              Separately blocks detected secrets on supported provider-bound
              channels. If an enabled channel cannot be inspected, it blocks
              instead of claiming protection.
            </p>
          </div>
          <label className="switch-control">
            <span>{data.outgoingGuardEnabled ? "Enabled" : "Disabled"}</span>
            <input
              type="checkbox"
              role="switch"
              checked={data.outgoingGuardEnabled}
              disabled={busy}
              onChange={() => void toggleGuard()}
              aria-label="Toggle outgoing secret guard"
            />
          </label>
        </article>
        <article>
          <div className="guard-controls__icon">
            <Radio aria-hidden="true" />
          </div>
          <div>
            <h2>Proxy capture fallback</h2>
            <p>
              Optional local fallback for facts native sources cannot establish.
              It may inspect provider-bound content and requires a separate
              explicit consent.
            </p>
          </div>
          {data.proxyEnabled ? (
            <Button
              variant="danger"
              disabled={busy}
              onClick={() => void disableProxy()}
            >
              Stop proxy
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void beginProxy()}
            >
              Review before enabling
            </Button>
          )}
        </article>
      </section>

      <section className="panel" aria-labelledby="coverage-heading">
        <div className="panel__header">
          <div>
            <p className="eyebrow">Current channel matrix</p>
            <h2 id="coverage-heading">Coverage</h2>
          </div>
          <span className="quiet-label">Unavailable ≠ zero findings</span>
        </div>
        <div className="coverage-grid">
          {data.channels.map((channel) => (
            <article className="coverage-card" key={channel.id}>
              <div className="coverage-card__heading">
                {channel.state === "inspected" ||
                channel.state === "blocked" ? (
                  <ShieldCheck aria-hidden="true" />
                ) : (
                  <EyeOff aria-hidden="true" />
                )}
                <div>
                  <strong>{channel.name}</strong>
                  <span>{channel.category.replaceAll("_", " ")}</span>
                </div>
                <StatusPill
                  state={
                    channel.state === "inspected" ? "healthy" : channel.state
                  }
                >
                  {channel.state}
                </StatusPill>
              </div>
              <p>{channel.description}</p>
              <div className="coverage-card__finding">
                <span>Sanitized findings</span>
                <strong>
                  {channel.findings === null
                    ? "Not inspectable"
                    : channel.findings}
                </strong>
              </div>
              {channel.limitation === null ? null : (
                <small>{channel.limitation}</small>
              )}
            </article>
          ))}
        </div>
      </section>

      <section
        className="context-boundary"
        aria-labelledby="context-boundary-heading"
      >
        <LockKeyhole aria-hidden="true" />
        <div>
          <h2 id="context-boundary-heading">
            Context breakdown is{" "}
            {data.contextBreakdownAvailable ? "available" : "absent"}
          </h2>
          <p>
            {data.contextBreakdownAvailable
              ? "The active consented proxy can attribute request composition. Each projection retains proxy provenance."
              : "Native channels do not expose request composition. Cutokyo shows no zero slices and makes no estimate while proxy capture is off."}
          </p>
        </div>
      </section>

      {preview === null ? null : (
        <Modal
          title="Enable local proxy capture?"
          description="Review exactly what this optional fallback may inspect before any listener starts."
          size="wide"
          onClose={() => setPreview(null)}
          footer={
            <>
              <Button onClick={() => setPreview(null)}>Not now</Button>
              <Button
                variant="primary"
                disabled={preview === "loading" || busy}
                onClick={() => void enableProxy()}
              >
                {busy ? "Starting…" : "I understand — enable proxy"}
              </Button>
            </>
          }
        >
          {preview === "loading" ? (
            <LoadingState label="Preparing proxy boundary preview" />
          ) : (
            <div className="consent-grid">
              <DefinitionList
                rows={[
                  { term: "Listener", value: preview.bindAddress },
                  {
                    term: "Native precedence",
                    value: preview.fallbackBehavior,
                  },
                  { term: "Outgoing guard", value: preview.guardBehavior },
                ]}
              />
              <section>
                <h3>Content that may be inspected</h3>
                <ul>
                  {preview.inspectedContent.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <section>
                <h3>Never persisted by the proxy</h3>
                <ul>
                  {preview.neverPersisted.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <Disclosure>
                Proxy capture observes provider traffic for attribution. It does
                not silently mutate provider payloads. Telemetry, on-disk,
                bundle, and provider-bound protections are separate controls.
              </Disclosure>
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

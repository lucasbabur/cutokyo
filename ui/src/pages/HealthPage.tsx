import {
  Archive,
  DatabaseZap,
  FileWarning,
  LockKeyhole,
  Stethoscope,
} from "lucide-react";
import { useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { BundlePreview, DoctorReport } from "../contracts.js";
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
  SuccessMessage,
  formatBytes,
  formatDateTime,
} from "../components/Primitives.js";

export function HealthPage() {
  const commands = useCommands();
  const resource = useCommandResource(() => commands.getHealth(), "health");
  const [busyDimension, setBusyDimension] = useState<string | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | "loading" | null>(null);
  const [bundle, setBundle] = useState<BundlePreview | "loading" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const announce = useAnnounce();

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Reading persisted health projection" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Health snapshot is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data;
  if (data === null) return null;
  const degraded = data.dimensions.filter(
    (dimension) => dimension.state === "degraded",
  );

  const retry = async (dimensionId: string) => {
    setBusyDimension(dimensionId);
    setError(null);
    try {
      const next = await commands.retryHealth(dimensionId);
      const remaining = next.dimensions.filter(
        (dimension) => dimension.state === "degraded",
      );
      const announcement = `${dimensionId.replaceAll("_", " ")} recovered. ${remaining.length} unrelated ${remaining.length === 1 ? "dimension remains" : "dimensions remain"} degraded.`;
      announce(announcement);
      setMessage(announcement);
      resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusyDimension(null);
    }
  };

  const runDoctor = async () => {
    setDoctor("loading");
    try {
      setDoctor(await commands.runDoctor());
    } catch (reason) {
      setDoctor(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const previewBundle = async () => {
    setBundle("loading");
    try {
      setBundle(await commands.previewBundle());
    } catch (reason) {
      setBundle(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const createBundle = async () => {
    const receipt = await commands.createBundle();
    setBundle(null);
    setMessage(receipt.message);
    announce(receipt.message);
  };

  return (
    <div className="page">
      <PageHeader
        eyebrow="Bounded persisted projection"
        title="System health"
        description="Writer lock, spool, quarantine, schema, integrity, rebuild, backup, and restore remain independent. Recovery in one dimension never clears another failure."
        actions={
          <>
            <Button onClick={() => void runDoctor()}>
              <Stethoscope aria-hidden="true" /> Run doctor
            </Button>
            <Button onClick={() => void previewBundle()}>
              <Archive aria-hidden="true" /> Bundle preview
            </Button>
          </>
        }
      />
      <RouteNotice meta={data.meta} />
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {message === null ? null : <SuccessMessage>{message}</SuccessMessage>}

      <section className="health-summary" aria-label="Health summary">
        <article>
          <StatusPill state={degraded.length === 0 ? "healthy" : "degraded"}>
            {degraded.length === 0 ? "Ready" : `${degraded.length} degraded`}
          </StatusPill>
          <strong>
            {degraded.length === 0
              ? "All bounded checks healthy"
              : "Attention is useful"}
          </strong>
          <span>Snapshot generation is independent of history size.</span>
        </article>
        <article>
          <FileWarning aria-hidden="true" />
          <div>
            <span>Quarantine</span>
            <strong>
              {data.currentQuarantineCount} current ·{" "}
              {data.lifetimeQuarantineCount} lifetime
            </strong>
          </div>
        </article>
        <article>
          <DatabaseZap aria-hidden="true" />
          <div>
            <span>Spool backlog</span>
            <strong>
              {data.drainPendingCount} files ·{" "}
              {formatBytes(data.drainPendingBytes)}
            </strong>
          </div>
        </article>
        <article>
          <LockKeyhole aria-hidden="true" />
          <div>
            <span>Writer owner</span>
            <strong>{data.writerOwner ?? "Unknown"}</strong>
          </div>
        </article>
      </section>

      {data.firstAffectedObservationId === null ? null : (
        <div className="first-affected" role="status">
          <FileWarning aria-hidden="true" />
          <div>
            <span>First currently affected observation</span>
            <code>{data.firstAffectedObservationId}</code>
          </div>
        </div>
      )}

      <section className="panel" aria-labelledby="dimension-heading">
        <div className="panel__header">
          <div>
            <p className="eyebrow">Independent dimensions</p>
            <h2 id="dimension-heading">Current and lifetime state</h2>
          </div>
          <span className="quiet-label">
            Schema {data.schemaVersion} · derive {data.deriveVersion}
          </span>
        </div>
        <div className="health-dimensions">
          {data.dimensions.map((dimension) => (
            <article
              key={dimension.id}
              className={`health-dimension health-dimension--${dimension.state}`}
            >
              <div className="health-dimension__title">
                <strong>{dimension.name}</strong>
                <StatusPill state={dimension.state}>
                  {dimension.state}
                </StatusPill>
              </div>
              <p>{dimension.detail}</p>
              <DefinitionList
                rows={[
                  {
                    term: "Last success",
                    value: formatDateTime(dimension.lastSuccessAt),
                  },
                  {
                    term: "Last failure",
                    value: formatDateTime(dimension.lastFailureAt),
                  },
                  {
                    term: "Category",
                    value: dimension.failureCategory ?? "None",
                  },
                ]}
              />
              {dimension.actionLabel === null ? null : (
                <Button
                  size="small"
                  variant={
                    dimension.id === "writer_lock" ? "primary" : "secondary"
                  }
                  disabled={busyDimension !== null}
                  onClick={() => void retry(dimension.id)}
                >
                  {busyDimension === dimension.id
                    ? "Retrying…"
                    : dimension.actionLabel}
                </Button>
              )}
            </article>
          ))}
        </div>
      </section>

      <section className="health-footnotes">
        <DefinitionList
          rows={[
            {
              term: "Last integrity result",
              value: data.lastIntegrityResult ?? "Not run",
            },
            {
              term: "Drain lag",
              value:
                data.drainLagSeconds === null
                  ? "Unknown"
                  : `${data.drainLagSeconds} seconds`,
            },
            { term: "Spool cap", value: data.spoolCapReason ?? "Not active" },
          ]}
        />
      </section>

      {doctor === null ? null : (
        <Modal
          title="Doctor report"
          description="The desktop and CLI read this same persisted health snapshot."
          size="wide"
          onClose={() => setDoctor(null)}
          footer={<Button onClick={() => setDoctor(null)}>Done</Button>}
        >
          {doctor === "loading" ? (
            <LoadingState label="Running bounded diagnostics" />
          ) : (
            <div>
              <StatusPill state={doctor.overall}>
                Overall {doctor.overall}
              </StatusPill>
              <ul className="doctor-list">
                {doctor.checks.map((check) => (
                  <li key={check.name}>
                    <StatusPill state={check.state}>{check.state}</StatusPill>
                    <div>
                      <strong>{check.name}</strong>
                      <span>{check.detail}</span>
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </Modal>
      )}

      {bundle === null ? null : (
        <Modal
          title="Diagnostic bundle preview"
          description="Review the bounded manifest before Cutokyo creates a local archive. Nothing is uploaded."
          size="wide"
          onClose={() => setBundle(null)}
          footer={
            <>
              <Button onClick={() => setBundle(null)}>Cancel</Button>
              <Button
                variant="primary"
                disabled={bundle === "loading"}
                onClick={() => void createBundle()}
              >
                Create local bundle
              </Button>
            </>
          }
        >
          {bundle === "loading" ? (
            <LoadingState label="Projecting safe bundle manifest" />
          ) : (
            <div className="bundle-preview">
              <DefinitionList
                rows={[
                  {
                    term: "Estimated size",
                    value: formatBytes(bundle.estimatedBytes),
                  },
                  { term: "Files", value: bundle.files.join(", ") },
                ]}
              />
              <section>
                <h3>Explicitly excluded</h3>
                <ul>
                  {bundle.exclusions.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <section>
                <h3>Sanitization</h3>
                <ul>
                  {bundle.redactions.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
              <Disclosure>
                Bundle redaction is not provider-bound request protection.
                Prompts, transcripts, raw secrets, and full project paths are
                omitted from this diagnostic artifact.
              </Disclosure>
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

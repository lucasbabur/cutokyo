import { Archive, Check, CircleDashed, Stethoscope } from "lucide-react";
import { useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { BundlePreview, DoctorReport } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  Button,
  DefinitionList,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  StatusPill,
  SuccessMessage,
  formatBytes,
  formatDateTime,
} from "../components/Primitives.js";
import { RouteNotice } from "../components/Notices.js";

export function HealthPage({
  onHealthChange,
}: {
  readonly onHealthChange?: () => void;
}) {
  const commands = useCommands();
  const resource = useCommandResource(() => commands.getHealth(), "health");
  const [busyDimension, setBusyDimension] = useState<string | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | "loading" | null>(null);
  const [bundle, setBundle] = useState<BundlePreview | "loading" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [bundleError, setBundleError] = useState<string | null>(null);
  const [bundleBusy, setBundleBusy] = useState(false);
  const retryInFlight = useRef(false);
  const bundleInFlight = useRef(false);
  const doctorRequest = useRef(0);
  const bundleRequest = useRef(0);
  const announce = useAnnounce();

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Loading health" />;
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
  const allHealthy =
    data.dimensions.length > 0 &&
    data.dimensions.every((dimension) => dimension.state === "healthy");

  const retry = async (dimensionId: string) => {
    if (retryInFlight.current) return;
    retryInFlight.current = true;
    setBusyDimension(dimensionId);
    setError(null);
    setMessage(null);
    try {
      const next = await commands.retryHealth(dimensionId);
      resource.reload();
      onHealthChange?.();
      const target = next.dimensions.find(
        (dimension) => dimension.id === dimensionId,
      );
      if (target?.state !== "healthy") {
        throw new Error(
          target?.detail ??
            "Recovery has not been verified. The health dimension remains unavailable.",
        );
      }
      const remaining = next.dimensions.filter(
        (dimension) => dimension.state === "degraded",
      );
      const announcement = `${target.name} recovered. ${remaining.length} unrelated ${remaining.length === 1 ? "dimension remains" : "dimensions remain"} degraded.`;
      announce(announcement);
      setMessage(announcement);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      retryInFlight.current = false;
      setBusyDimension(null);
    }
  };

  const closeDoctor = () => {
    doctorRequest.current += 1;
    setDoctor(null);
  };
  const closeBundle = () => {
    if (bundleInFlight.current) return;
    bundleRequest.current += 1;
    setBundle(null);
  };
  const runDoctor = async () => {
    const request = ++doctorRequest.current;
    setDoctor("loading");
    setError(null);
    try {
      const result = await commands.runDoctor();
      if (doctorRequest.current === request) setDoctor(result);
    } catch (reason) {
      if (doctorRequest.current !== request) return;
      setDoctor(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const previewBundle = async () => {
    const request = ++bundleRequest.current;
    setBundle("loading");
    setBundleError(null);
    setError(null);
    setMessage(null);
    try {
      const result = await commands.previewBundle();
      if (bundleRequest.current === request) setBundle(result);
    } catch (reason) {
      if (bundleRequest.current !== request) return;
      setBundle(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const createBundle = async () => {
    if (bundleInFlight.current) return;
    bundleInFlight.current = true;
    setBundleBusy(true);
    setBundleError(null);
    try {
      const receipt = await commands.createBundle();
      if (!receipt.ok || receipt.status !== "success")
        throw new Error(receipt.message);
      setBundle(null);
      setMessage(receipt.message);
      announce(receipt.message);
    } catch (reason) {
      setBundleError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      bundleInFlight.current = false;
      setBundleBusy(false);
    }
  };

  const attention = data.dimensions.filter((item) => item.state === "degraded");
  const quiet = data.dimensions.filter((item) => item.state !== "degraded");
  return (
    <div className="page">
      <PageHeader
        title="Health"
        actions={
          <>
            <Button
              disabled={doctor !== null || bundle !== null}
              onClick={() => void runDoctor()}
            >
              <Stethoscope aria-hidden="true" /> Run diagnostics
            </Button>
            <Button
              disabled={doctor !== null || bundle !== null}
              onClick={() => void previewBundle()}
            >
              <Archive aria-hidden="true" /> Export report
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
      {message === null ? null : (
        <div className="toast">
          <SuccessMessage>{message}</SuccessMessage>
        </div>
      )}

      <p className="health-line" role="status">
        {attention.length === 0
          ? allHealthy
            ? "All systems healthy"
            : "Some checks have not run yet"
          : `${attention.length} ${attention.length === 1 ? "issue" : "issues"}`}
      </p>

      {attention.length === 0 ? null : (
        <div className="health-dimensions">
          {attention.map((dimension) => (
            <article
              key={dimension.id}
              className={`health-dimension health-dimension--${dimension.state}`}
            >
              <strong>{dimension.name}</strong>
              <p>{dimension.detail}</p>
              <div className="control-row">
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
              </div>
              <details className="advanced">
                <summary>Details</summary>
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
              </details>
            </article>
          ))}
        </div>
      )}

      {quiet.length === 0 ? null : (
        <ul className="health-checklist" aria-label="Other checks">
          {quiet.map((dimension) => (
            <li key={dimension.id}>
              {dimension.state === "healthy" ? (
                <Check aria-hidden="true" />
              ) : (
                <CircleDashed aria-hidden="true" />
              )}
              <span>{dimension.name}</span>
              {dimension.state === "healthy" ? null : (
                <span className="sr-only"> not checked yet</span>
              )}
            </li>
          ))}
        </ul>
      )}

      <details className="advanced">
        <summary>Technical details</summary>
        <DefinitionList
          rows={[
            {
              term: "Quarantine",
              value: `${data.currentQuarantineCount} current · ${data.lifetimeQuarantineCount} lifetime`,
            },
            {
              term: "Waiting files",
              value: `${data.drainPendingCount} files · ${formatBytes(data.drainPendingBytes)}`,
            },
            { term: "Writer owner", value: data.writerOwner ?? "Unknown" },
            ...(data.firstAffectedObservationId === null
              ? []
              : [
                  {
                    term: "First affected record",
                    value: <code>{data.firstAffectedObservationId}</code>,
                  },
                ]),
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
            {
              term: "Storage limit",
              value: data.spoolCapReason ?? "Not active",
            },
            {
              term: "Schema",
              value: `${data.schemaVersion} · derive ${data.deriveVersion}`,
            },
          ]}
        />
      </details>

      {doctor === null ? null : (
        <Modal
          title="Diagnostics"
          size="wide"
          onClose={closeDoctor}
          footer={<Button onClick={closeDoctor}>Done</Button>}
        >
          {doctor === "loading" ? (
            <LoadingState label="Running" />
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
          title="Export report"
          size="wide"
          onClose={closeBundle}
          closeDisabled={bundleBusy}
          footer={
            <>
              <Button disabled={bundleBusy} onClick={closeBundle}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={bundle === "loading" || bundleBusy}
                onClick={() => void createBundle()}
              >
                {bundleBusy ? "Exporting…" : "Export"}
              </Button>
            </>
          }
        >
          {bundleError === null ? null : (
            <p className="form-error" role="alert">
              {bundleError}
            </p>
          )}
          {bundle === "loading" ? (
            <LoadingState label="Preparing" />
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
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

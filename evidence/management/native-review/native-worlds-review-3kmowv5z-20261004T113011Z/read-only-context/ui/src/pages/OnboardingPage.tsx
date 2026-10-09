import {
  ArrowRight,
  HardDrive,
  LockKeyhole,
  Radar,
  ShieldCheck,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  CaptureSetupOperation,
  CaptureSetupPreview,
  Harness,
} from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import { navigate } from "../router.js";
import {
  Button,
  CoverageBadge,
  Disclosure,
  ErrorState,
  LoadingState,
  PageHeader,
  RouteNotice,
} from "../components/Primitives.js";

const HARNESS_NAMES: Record<Harness, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
};
const REMOVAL_DISCLOSURE =
  "Removal changes configuration only. Already-running harness callbacks may remain loaded until that harness restarts; live capture remains unknown.";

export function OnboardingPage({
  mode,
  onComplete,
}: {
  readonly mode: "onboarding" | "management";
  readonly onComplete: () => void;
}) {
  const management = mode === "management";
  const HarnessChoice = management ? "div" : "label";
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getOnboarding(),
    "onboarding",
  );
  const [selected, setSelected] = useState<ReadonlySet<Harness>>(
    () => new Set<Harness>(),
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const mutationPending = useRef(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [preview, setPreview] = useState<CaptureSetupPreview | null>(null);
  const [verified, setVerified] = useState<ReadonlySet<Harness>>(
    () => new Set(),
  );
  const [setupMessage, setSetupMessage] = useState<string | null>(null);
  const announce = useAnnounce();

  const selectedList = useMemo(() => [...selected], [selected]);

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Checking local harness coverage" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Setup status is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const data = resource.data;
  if (data === null) return null;

  const toggleHarness = (harness: Harness) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(harness)) next.delete(harness);
      else next.add(harness);
      return next;
    });
  };

  const inspect = async (
    harness: Harness,
    operation: CaptureSetupOperation,
  ) => {
    if (mutationPending.current) return;
    mutationPending.current = true;
    setSubmitting(true);
    setPreview(null);
    setSubmitError(null);
    try {
      const plan = await commands.previewCaptureSetup(harness, operation);
      setPreview(plan);
      setVerified((current) => {
        const next = new Set(current);
        if (plan.verified) next.add(harness);
        else next.delete(harness);
        return next;
      });
    } catch (reason) {
      setSubmitError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      mutationPending.current = false;
      setSubmitting(false);
    }
  };

  const apply = async () => {
    if (preview === null) return;
    if (mutationPending.current) return;
    mutationPending.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await commands.applyCaptureSetup(preview.previewToken);
      setVerified((current) => {
        const next = new Set(current);
        if (result.verified) next.add(result.harness);
        else next.delete(result.harness);
        return next;
      });
      setSetupMessage(result.message);
      announce(result.message);
      setPreview(null);
      resource.reload();
    } catch (reason) {
      setPreview(null);
      setVerified((current) => {
        const next = new Set(current);
        next.delete(preview.harness);
        return next;
      });
      setSubmitError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      mutationPending.current = false;
      setSubmitting(false);
    }
  };

  const complete = async (mode: "install" | "browse") => {
    if (mutationPending.current) return;
    mutationPending.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await commands.completeOnboarding({
        mode,
        harnesses: mode === "browse" ? [] : selectedList,
        acknowledgedPlaintextStorage: acknowledged,
        proxyEnabled: false,
        analysisEgressEnabled: false,
      });
      announce(result.message);
      onComplete();
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      setSubmitError(message);
      announce(`Setup could not finish: ${message}`);
    } finally {
      mutationPending.current = false;
      setSubmitting(false);
    }
  };

  return (
    <div className="page page--onboarding">
      <PageHeader
        eyebrow="Private by default"
        title={
          management
            ? "Manage native capture"
            : "See your agent work without sending it away"
        }
        description={
          management
            ? "Preview installation, recover an interrupted operation, or remove only Cutokyo-owned capture configuration. Managing capture preserves your onboarding choice, selected harness preferences, history, and unrelated settings."
            : "Cutokyo reads supported native surfaces, keeps evidence on this device, and labels every gap. Choose the harnesses you want to observe."
        }
      />
      <RouteNotice meta={data.meta} />

      <div className="onboarding-layout">
        <section className="setup-panel" aria-labelledby="setup-heading">
          <div className="section-heading">
            <div>
              <p className="eyebrow">
                {management ? "Native integrations" : "Step 1 of 2"}
              </p>
              <h2 id="setup-heading">
                {management
                  ? "Preview capture changes"
                  : "Preview and install native capture"}
              </h2>
            </div>
            <span className="quiet-label">No proxy</span>
          </div>
          <div className="harness-choice-list">
            {data.harnesses.map((item) => (
              <div
                key={item.harness}
                role="group"
                aria-label={`${HARNESS_NAMES[item.harness]} capture configuration`}
              >
                <HarnessChoice className="harness-choice">
                  {management ? null : (
                    <input
                      type="checkbox"
                      checked={selected.has(item.harness)}
                      onChange={() => toggleHarness(item.harness)}
                    />
                  )}
                  <span className="harness-choice__body">
                    <span className="harness-choice__title">
                      <strong>{HARNESS_NAMES[item.harness]}</strong>
                      <CoverageBadge coverage={item.capture} />
                    </span>
                    <span>{item.source}</span>
                    {management || item.actionable === null ? null : (
                      <span className="harness-choice__note">
                        {item.actionable}
                      </span>
                    )}
                  </span>
                </HarnessChoice>
                <div className="setup-actions">
                  <Button
                    disabled={submitting}
                    onClick={() => void inspect(item.harness, "install")}
                  >
                    Preview {HARNESS_NAMES[item.harness]} install
                  </Button>
                  <Button
                    disabled={submitting}
                    onClick={() => void inspect(item.harness, "recover")}
                  >
                    Recover {HARNESS_NAMES[item.harness]}
                  </Button>
                  <Button
                    disabled={submitting}
                    onClick={() => void inspect(item.harness, "uninstall")}
                  >
                    Preview {HARNESS_NAMES[item.harness]} removal
                  </Button>
                  <span>
                    {verified.has(item.harness)
                      ? "Configuration verified. Live capture unknown."
                      : "Configuration not verified. Live capture unknown."}
                  </span>
                </div>
              </div>
            ))}
          </div>

          {preview === null ? null : (
            <section className="panel" aria-label="Capture setup preview">
              <h3>
                {HARNESS_NAMES[preview.harness]} {preview.operation} preview
              </h3>
              <p>Installed version: {preview.version}</p>
              <ul>
                {preview.targets.map((target) => (
                  <li key={target}>{target}</li>
                ))}
              </ul>
              <ul>
                {preview.actions.map((action, index) => (
                  <li key={`${index}-${action}`}>{action}</li>
                ))}
              </ul>
              {preview.issues.map((issue) => (
                <p key={issue}>{issue}</p>
              ))}
              <Disclosure>{preview.disclosure}</Disclosure>
              {preview.operation === "uninstall" ? (
                <Disclosure>{REMOVAL_DISCLOSURE}</Disclosure>
              ) : null}
              <Button
                disabled={submitting || !acknowledged}
                variant="primary"
                onClick={() => void apply()}
              >
                Confirm {preview.operation} for {HARNESS_NAMES[preview.harness]}
              </Button>
              <Button disabled={submitting} onClick={() => setPreview(null)}>
                Cancel preview
              </Button>
              {!acknowledged ? (
                <p>
                  Acknowledge local storage below before changing capture
                  configuration.
                </p>
              ) : null}
            </section>
          )}
          {setupMessage === null ? null : <p role="status">{setupMessage}</p>}
          <div className="setup-divider" />

          <div className="section-heading">
            <div>
              <p className="eyebrow">
                {management ? "Change confirmation" : "Step 2 of 2"}
              </p>
              <h2>Confirm local storage</h2>
            </div>
          </div>
          <label className="acknowledgement">
            <input
              type="checkbox"
              checked={acknowledged}
              onChange={(event) => setAcknowledged(event.currentTarget.checked)}
            />
            <span>
              <strong>I understand the v0.x storage boundary</strong>
              <span>{data.storageDisclosure}</span>
            </span>
          </label>
          <Disclosure>{data.telemetryDisclosure}</Disclosure>
          {submitError === null ? null : (
            <p className="form-error" role="alert">
              {submitError}
            </p>
          )}
          {management ? (
            <div className="setup-actions">
              <Button
                disabled={submitting}
                onClick={() => navigate("/settings")}
              >
                Return to Settings
              </Button>
              <span>
                Selected harness preferences are unchanged. {REMOVAL_DISCLOSURE}
              </span>
            </div>
          ) : (
            <div className="setup-actions">
              <Button
                variant="primary"
                disabled={
                  !acknowledged ||
                  selected.size === 0 ||
                  selectedList.some((harness) => !verified.has(harness)) ||
                  submitting
                }
                onClick={() => void complete("install")}
              >
                {submitting ? "Working…" : "Finish with verified capture"}
                <ArrowRight aria-hidden="true" />
              </Button>
              <Button
                disabled={!acknowledged || submitting}
                onClick={() => void complete("browse")}
              >
                Browse without installing
              </Button>
              <span>
                Browse-only does not install or remove capture. Proxy capture
                and AI analysis stay disabled.
              </span>
            </div>
          )}
        </section>

        <aside className="privacy-rail" aria-label="Privacy boundaries">
          <div className="privacy-rail__header">
            <ShieldCheck aria-hidden="true" />
            <div>
              <p className="eyebrow">What happens next</p>
              <h2>Three clear boundaries</h2>
            </div>
          </div>
          <ol className="boundary-list">
            <li>
              <Radar aria-hidden="true" />
              <div>
                <strong>Native capture first</strong>
                <span>
                  Hooks, local APIs, OTel, and local state precede any fallback.
                </span>
              </div>
            </li>
            <li>
              <HardDrive aria-hidden="true" />
              <div>
                <strong>One local evidence store</strong>
                <span>
                  Owner-only files, durable spool, searchable SQLite, no cloud
                  account.
                </span>
              </div>
            </li>
            <li>
              <LockKeyhole aria-hidden="true" />
              <div>
                <strong>Egress only when asked</strong>
                <span>
                  Proxy and analysis each show a separate preview before
                  consent.
                </span>
              </div>
            </li>
          </ol>
        </aside>
      </div>
    </div>
  );
}

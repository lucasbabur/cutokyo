import {
  ArrowRight,
  HardDrive,
  LockKeyhole,
  Radar,
  ShieldCheck,
} from "lucide-react";
import { useMemo, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { Harness } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
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

export function OnboardingPage({
  onComplete,
}: {
  readonly onComplete: () => void;
}) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getOnboarding(),
    "onboarding",
  );
  const [selected, setSelected] = useState<ReadonlySet<Harness>>(
    () => new Set<Harness>(["claude_code", "codex", "opencode"]),
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
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

  const complete = async () => {
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await commands.completeOnboarding({
        harnesses: selectedList,
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
      setSubmitting(false);
    }
  };

  return (
    <div className="page page--onboarding">
      <PageHeader
        eyebrow="Private by default"
        title="See your agent work without sending it away"
        description="Cutokyo reads supported native surfaces, keeps evidence on this device, and labels every gap. Choose the harnesses you want to observe."
      />
      <RouteNotice meta={data.meta} />

      <div className="onboarding-layout">
        <section className="setup-panel" aria-labelledby="setup-heading">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Step 1 of 2</p>
              <h2 id="setup-heading">Select local capture surfaces</h2>
            </div>
            <span className="quiet-label">No proxy</span>
          </div>
          <div className="harness-choice-list">
            {data.harnesses.map((item) => (
              <label className="harness-choice" key={item.harness}>
                <input
                  type="checkbox"
                  checked={selected.has(item.harness)}
                  onChange={() => toggleHarness(item.harness)}
                />
                <span className="harness-choice__body">
                  <span className="harness-choice__title">
                    <strong>{HARNESS_NAMES[item.harness]}</strong>
                    <CoverageBadge coverage={item.capture} />
                  </span>
                  <span>{item.source}</span>
                  {item.actionable === null ? null : (
                    <span className="harness-choice__note">
                      {item.actionable}
                    </span>
                  )}
                </span>
              </label>
            ))}
          </div>

          <div className="setup-divider" />

          <div className="section-heading">
            <div>
              <p className="eyebrow">Step 2 of 2</p>
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
          <div className="setup-actions">
            <Button
              variant="primary"
              disabled={!acknowledged || selected.size === 0 || submitting}
              onClick={() => void complete()}
            >
              {submitting ? "Setting up…" : "Complete local-only setup"}
              <ArrowRight aria-hidden="true" />
            </Button>
            <span>Proxy capture and AI analysis stay disabled.</span>
          </div>
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

import {
  Bot,
  CheckCircle2,
  FileSearch,
  ShieldCheck,
  Sparkles,
  XCircle,
} from "lucide-react";
import { useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { AnalysisPreview, AnalysisResult } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  Button,
  CoverageBadge,
  DefinitionList,
  Disclosure,
  EmptyState,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
} from "../components/Primitives.js";

export function AnalysisPage() {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getAnalysisCandidates(),
    "analysis-candidates",
  );
  const [preview, setPreview] = useState<AnalysisPreview | "loading" | null>(
    null,
  );
  const [phase, setPhase] = useState<"idle" | "running" | "error" | "success">(
    "idle",
  );
  const [result, setResult] = useState<AnalysisResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runGeneration = useRef(0);
  const announce = useAnnounce();

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Reading locally eligible sessions" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Analysis candidates are unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const candidates = resource.data;
  if (candidates === null) return null;

  const beginPreview = async (sessionIds: readonly string[]) => {
    setPreview("loading");
    setPhase("idle");
    setError(null);
    try {
      setPreview(await commands.previewAnalysis(sessionIds));
    } catch (reason) {
      setPreview(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const cancelPreview = async () => {
    if (preview !== null && preview !== "loading") {
      await commands.cancelAnalysis(preview.requestId);
      announce("Analysis preview closed. No provider request was sent.");
    }
    setPreview(null);
    setPhase("idle");
  };

  const run = async () => {
    if (preview === null || preview === "loading") return;
    const generation = runGeneration.current + 1;
    runGeneration.current = generation;
    setPhase("running");
    setError(null);
    try {
      const next = await commands.runAnalysis(preview.previewToken);
      if (runGeneration.current !== generation) return;
      setResult(next);
      setPhase("success");
      setPreview(null);
      announce("AI analysis finished and its attributable summary was saved.");
    } catch (reason) {
      if (runGeneration.current !== generation) return;
      setError(reason instanceof Error ? reason.message : String(reason));
      setPhase("error");
    }
  };

  const cancelRunning = async () => {
    if (preview === null || preview === "loading") return;
    runGeneration.current += 1;
    await commands.cancelAnalysis(preview.requestId);
    setPhase("idle");
    setPreview(null);
    announce("Analysis cancelled. No summary was written.");
  };

  return (
    <div className="page">
      <PageHeader
        eyebrow="Explicit, attributable egress"
        title="AI analysis"
        description="Nothing is sent when this page opens. Every request previews selected sessions, redacted payload scope, provider, model, price uncertainty, and prompt contract before confirmation."
      />

      <section
        className="analysis-boundary"
        aria-labelledby="analysis-boundary-heading"
      >
        <ShieldCheck aria-hidden="true" />
        <div>
          <h2 id="analysis-boundary-heading">Confirmation is per request</h2>
          <p>
            Redaction runs before egress. Credentials remain in the OS keychain.
            Cancellation reaches the application request rather than merely
            closing this screen.
          </p>
        </div>
        <span className="quiet-label">No automatic analysis</span>
      </section>

      {error === null || phase === "error" ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {result === null ? null : (
        <section className="summary-card" aria-labelledby="summary-heading">
          <div className="summary-card__header">
            <CheckCircle2 aria-hidden="true" />
            <div>
              <p className="eyebrow">Saved attributable summary</p>
              <h2 id="summary-heading">Analysis complete</h2>
            </div>
          </div>
          <p>{result.text}</p>
          <DefinitionList
            rows={[
              {
                term: "Provider / model",
                value: `${result.provider} / ${result.model}`,
              },
              { term: "Prompt version", value: result.promptVersion },
              {
                term: "Source sessions",
                value: result.sourceSessionIds.join(", "),
              },
              {
                term: "Idempotency key",
                value: <code>{result.idempotencyKey}</code>,
              },
            ]}
          />
        </section>
      )}

      <section className="panel" aria-labelledby="candidate-heading">
        <div className="panel__header">
          <div>
            <p className="eyebrow">Local selection</p>
            <h2 id="candidate-heading">Sessions eligible for preview</h2>
          </div>
          <FileSearch aria-hidden="true" />
        </div>
        {candidates.length === 0 ? (
          <EmptyState
            compact
            title="No sessions available for analysis"
            description="Capture a session first. Cutokyo will not send an empty placeholder or infer missing transcript content."
          />
        ) : (
          <div className="candidate-list">
            {candidates.map((candidate) => (
              <article key={candidate.sessionId}>
                <div>
                  <strong>{candidate.title}</strong>
                  <span>
                    {candidate.harness.replaceAll("_", " ")} ·{" "}
                    <code>{candidate.sessionId}</code>
                  </span>
                </div>
                <CoverageBadge coverage={candidate.coverage} />
                <Button
                  size="small"
                  onClick={() => void beginPreview([candidate.sessionId])}
                >
                  <Sparkles aria-hidden="true" /> Analyze
                </Button>
              </article>
            ))}
          </div>
        )}
      </section>

      {preview === null ? null : (
        <Modal
          title={
            phase === "running"
              ? "Analysis in progress"
              : phase === "error"
                ? "Provider request failed"
                : "Review AI analysis egress"
          }
          description={
            phase === "running"
              ? "The confirmed redacted request is in flight. You can still cancel it."
              : "Nothing leaves this device until you confirm this exact preview."
          }
          size="wide"
          onClose={() =>
            void (phase === "running" ? cancelRunning() : cancelPreview())
          }
          footer={
            phase === "running" ? (
              <Button variant="danger" onClick={() => void cancelRunning()}>
                Cancel request
              </Button>
            ) : phase === "error" ? (
              <>
                <Button onClick={() => void cancelPreview()}>Close</Button>
                <Button variant="primary" onClick={() => void run()}>
                  Retry same idempotent request
                </Button>
              </>
            ) : (
              <>
                <Button onClick={() => void cancelPreview()}>
                  Cancel — send nothing
                </Button>
                <Button
                  variant="primary"
                  disabled={preview === "loading"}
                  onClick={() => void run()}
                >
                  Confirm and send redacted payload
                </Button>
              </>
            )
          }
        >
          {preview === "loading" ? (
            <LoadingState label="Preparing a redacted local preview" />
          ) : phase === "running" ? (
            <div className="analysis-progress" role="status" aria-live="polite">
              <div className="progress-orbit" aria-hidden="true">
                <Bot />
              </div>
              <strong>Waiting for {preview.provider}</strong>
              <p>
                The application will write one idempotent summary only after a
                complete response.
              </p>
              <div className="indeterminate-track">
                <span />
              </div>
            </div>
          ) : phase === "error" ? (
            <div className="analysis-error" role="alert">
              <XCircle aria-hidden="true" />
              <div>
                <strong>No summary was written</strong>
                <p>{error}</p>
                <p>Retry reuses the same idempotency key.</p>
              </div>
            </div>
          ) : (
            <div className="analysis-preview">
              <DefinitionList
                rows={[
                  {
                    term: "Selected sessions",
                    value: preview.sourceSessionTitles.join(", "),
                  },
                  {
                    term: "Source IDs",
                    value: preview.sourceSessionIds.join(", "),
                  },
                  { term: "Provider", value: preview.provider },
                  { term: "Model", value: preview.model },
                  { term: "Prompt contract", value: preview.promptVersion },
                  {
                    term: "Estimated input",
                    value:
                      preview.estimatedInputTokens === null
                        ? "Unknown"
                        : `${preview.estimatedInputTokens.toLocaleString()} tokens`,
                  },
                  {
                    term: "Price",
                    value:
                      preview.estimatedPriceMicros === null
                        ? "Unknown"
                        : `$${(preview.estimatedPriceMicros / 1_000_000).toFixed(2)}`,
                  },
                ]}
              />
              <div className="consent-grid">
                <section>
                  <h3>Payload scope</h3>
                  <ul>
                    {preview.payloadScope.map((item) => (
                      <li key={item}>{item}</li>
                    ))}
                  </ul>
                </section>
                <section>
                  <h3>Redaction before send</h3>
                  <ul>
                    {preview.redactions.map((item) => (
                      <li key={item}>{item}</li>
                    ))}
                  </ul>
                </section>
              </div>
              <Disclosure>{preview.priceLabel}</Disclosure>
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}

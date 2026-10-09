import { Check, MoreHorizontal } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

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
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
} from "../components/Primitives.js";
import { HARNESS_NAMES, HarnessLabel } from "../components/HarnessMark.js";
import { RouteNotice } from "../components/Notices.js";

const OPERATION_LABEL: Readonly<Record<CaptureSetupOperation, string>> = {
  install: "Install",
  recover: "Repair",
  uninstall: "Remove",
};

function RowMenu({
  harness,
  disabled,
  onChoose,
}: {
  readonly harness: Harness;
  readonly disabled: boolean;
  readonly onChoose: (operation: CaptureSetupOperation) => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent) {
        if (event.key !== "Escape") return;
        event.preventDefault();
        setOpen(false);
        trigger.current?.focus();
      } else if (!root.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("keydown", close);
    document.addEventListener("pointerdown", close);
    return () => {
      document.removeEventListener("keydown", close);
      document.removeEventListener("pointerdown", close);
    };
  }, [open]);
  return (
    <div className="row-menu" ref={root}>
      <Button
        ref={trigger}
        variant="quiet"
        size="small"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`More actions for ${HARNESS_NAMES[harness]}`}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        <MoreHorizontal aria-hidden="true" />
      </Button>
      {open ? (
        <ul className="row-menu__list" role="menu">
          {(["install", "recover", "uninstall"] as const).map((operation) => (
            <li key={operation} role="none">
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  onChoose(operation);
                }}
              >
                {OPERATION_LABEL[operation]} {HARNESS_NAMES[harness]}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

export function OnboardingPage({
  mode,
  onComplete,
  statusRefresh = 0,
}: {
  readonly mode: "onboarding" | "management";
  readonly onComplete: () => void;
  readonly statusRefresh?: number;
}) {
  const management = mode === "management";
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getOnboarding(),
    `onboarding:${statusRefresh}`,
  );
  const [selected, setSelected] = useState<ReadonlySet<Harness>>(
    () => new Set<Harness>(),
  );
  const [queue, setQueue] = useState<readonly Harness[]>([]);
  const [acknowledged, setAcknowledged] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const mutationPending = useRef(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [preview, setPreview] = useState<CaptureSetupPreview | null>(null);
  const [verified, setVerified] = useState<ReadonlySet<Harness>>(
    () => new Set(),
  );
  const announce = useAnnounce();

  const selectedList = useMemo(() => [...selected], [selected]);

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Checking your agents" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Setup is unavailable"
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
      setQueue([]);
      setSubmitError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      mutationPending.current = false;
      setSubmitting(false);
    }
  };

  const cancelPreview = () => {
    if (submitting) return;
    setPreview(null);
    setQueue([]);
  };

  const apply = async () => {
    if (preview === null) return;
    if (mutationPending.current) return;
    mutationPending.current = true;
    setSubmitting(true);
    setSubmitError(null);
    let following: Harness | undefined;
    try {
      const result = await commands.applyCaptureSetup(preview.previewToken);
      setVerified((current) => {
        const next = new Set(current);
        if (result.verified) next.add(result.harness);
        else next.delete(result.harness);
        return next;
      });
      announce(result.message);
      setPreview(null);
      const remaining = queue.filter((entry) => entry !== result.harness);
      setQueue(remaining);
      following = remaining[0];
      resource.reload();
    } catch (reason) {
      setPreview(null);
      setQueue([]);
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
    if (following !== undefined) void inspect(following, "install");
  };

  const complete = async (choice: "install" | "browse") => {
    if (mutationPending.current) return;
    mutationPending.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await commands.completeOnboarding({
        mode: choice,
        harnesses: choice === "browse" ? [] : selectedList,
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

  const pending = selectedList.filter((harness) => !verified.has(harness));
  const allReady = selectedList.length > 0 && pending.length === 0;
  const startInstall = () => {
    setQueue(pending);
    const first = pending[0];
    if (first !== undefined) void inspect(first, "install");
  };

  return (
    <div className="page page--onboarding">
      <PageHeader title={management ? "Manage capture" : "Set up Cutokyo"} />
      <RouteNotice meta={data.meta} />
      {management ? null : (
        <p className="page-line">Choose agents to capture.</p>
      )}
      {resource.state === "error" ? (
        <ErrorState
          title="Setup could not refresh"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}

      <ul className="harness-rows" aria-label="Agents">
        {data.harnesses.map((item) => (
          <li
            key={item.harness}
            className="harness-row"
            aria-label={`${HARNESS_NAMES[item.harness]} capture`}
          >
            {management ? null : (
              <input
                type="checkbox"
                aria-label={`Capture ${HARNESS_NAMES[item.harness]}`}
                checked={selected.has(item.harness)}
                onChange={() => toggleHarness(item.harness)}
              />
            )}
            <strong className="harness-row__name">
              <HarnessLabel harness={item.harness} size={20} />
            </strong>
            <span className="harness-row__status">
              {verified.has(item.harness) ? (
                <>
                  <Check aria-hidden="true" /> Ready
                </>
              ) : null}
            </span>
            <RowMenu
              harness={item.harness}
              disabled={submitting}
              onChoose={(operation) => void inspect(item.harness, operation)}
            />
          </li>
        ))}
      </ul>

      {submitError === null ? null : (
        <p className="form-error" role="alert">
          {submitError}
        </p>
      )}

      {management ? (
        <div className="control-row">
          <Button disabled={submitting} onClick={() => navigate("/settings")}>
            Return to Settings
          </Button>
        </div>
      ) : (
        <div className="control-row">
          {allReady ? (
            <Button
              variant="primary"
              disabled={!acknowledged || submitting}
              onClick={() => void complete("install")}
            >
              {submitting ? "Working…" : "Continue"}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={
                selectedList.length === 0 || !acknowledged || submitting
              }
              onClick={startInstall}
            >
              Install selected
            </Button>
          )}
          {selectedList.length > 0 ? null : (
            <Button
              disabled={!acknowledged || submitting}
              onClick={() => void complete("browse")}
            >
              Skip for now
            </Button>
          )}
          <label className="check-row">
            <input
              type="checkbox"
              checked={acknowledged}
              onChange={(event) => setAcknowledged(event.currentTarget.checked)}
            />
            <span>History is stored unencrypted on this device.</span>
          </label>
        </div>
      )}

      {preview === null ? null : (
        <Modal
          title={`${OPERATION_LABEL[preview.operation]} ${HARNESS_NAMES[preview.harness]}?`}
          onClose={cancelPreview}
          closeDisabled={submitting}
          footer={
            <>
              <Button disabled={submitting} onClick={cancelPreview}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={submitting}
                onClick={() => void apply()}
              >
                Confirm
              </Button>
            </>
          }
        >
          <ul className="note-list" aria-label="Files that change">
            {preview.targets.map((target) => (
              <li key={target}>
                <code>{target}</code>
              </li>
            ))}
          </ul>
          {preview.issues.map((issue) => (
            <p key={issue} className="form-error">
              {issue}
            </p>
          ))}
        </Modal>
      )}
    </div>
  );
}

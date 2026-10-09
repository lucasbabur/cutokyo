import { CalendarClock, Database } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { RetentionPreview, SessionFilters } from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import { BackupControls } from "../components/BackupControls.js";
import { DEFAULT_SESSION_FILTERS } from "../domain/sessionSearch.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  SuccessMessage,
} from "../components/Primitives.js";

const ALL_SESSIONS: SessionFilters = DEFAULT_SESSION_FILTERS;

export function DataControlsPage() {
  const commands = useCommands();
  const history = useCommandResource(
    () => commands.searchSessions(ALL_SESSIONS),
    "data-history",
  );
  const settings = useCommandResource(
    () => commands.getSettings(),
    "data-retention-settings",
  );
  const [retentionChoice, setRetentionChoice] = useState<
    number | null | undefined
  >(undefined);
  const days =
    retentionChoice === undefined
      ? (settings.data?.retention_days ?? null)
      : retentionChoice;
  const [policyBusy, setPolicyBusy] = useState(false);
  const [retention, setRetention] = useState<
    RetentionPreview | "loading" | "failed" | null
  >(null);
  const [deleteAllOpen, setDeleteAllOpen] = useState(false);
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => {
    if (message === null) return;
    const timer = globalThis.setTimeout(() => setMessage(null), 6000);
    return () => globalThis.clearTimeout(timer);
  }, [message]);
  const [error, setError] = useState<string | null>(null);
  const [modalError, setModalError] = useState<string | null>(null);
  const running = useRef(false);
  const policyRunning = useRef(false);
  const previewGeneration = useRef(0);
  const announce = useAnnounce();
  const closeRetention = () => {
    if (running.current) return;
    previewGeneration.current += 1;
    setRetention(null);
    setModalError(null);
  };
  const closeDeleteAll = () => {
    if (running.current) return;
    setDeleteAllOpen(false);
    setConfirmation("");
    setModalError(null);
  };

  if (history.state === "loading" && history.data === null) {
    return <LoadingState label="Counting linked local history" />;
  }
  if (history.state === "error" && history.data === null) {
    return (
      <ErrorState
        title="Data controls are unavailable"
        error={history.error}
        onRetry={history.reload}
      />
    );
  }
  const data = history.data;
  if (data === null) return null;

  const saveRetentionPolicy = async () => {
    if (policyRunning.current || settings.data === null) return;
    policyRunning.current = true;
    setPolicyBusy(true);
    setError(null);
    try {
      await commands.patchSettings({ retention_days: days });
      const next =
        days === null
          ? "Retention policy saved: keep until explicitly deleted. No history was deleted."
          : `Retention policy saved: ${days} days. No history was deleted; review a cleanup preview separately.`;
      setMessage(next);
      announce(next);
      settings.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      policyRunning.current = false;
      setPolicyBusy(false);
    }
  };
  const previewRetention = async () => {
    if (days === null || retention !== null || running.current) return;
    const generation = ++previewGeneration.current;
    setRetention("loading");
    setModalError(null);
    try {
      const preview = await commands.previewRetention(days);
      if (generation === previewGeneration.current) setRetention(preview);
    } catch (reason) {
      if (generation !== previewGeneration.current) return;
      setRetention("failed");
      setModalError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const applyRetention = async () => {
    if (retention === null || typeof retention === "string" || running.current)
      return;
    running.current = true;
    setModalError(null);
    setBusy(true);
    try {
      const receipt = await commands.applyRetention(retention.previewToken);
      const next = `Retention applied exactly as previewed: ${receipt.sessions} ${receipt.sessions === 1 ? "session" : "sessions"} and ${receipt.ftsRows} search ${receipt.ftsRows === 1 ? "row" : "rows"} deleted.`;
      setMessage(next);
      announce(next);
      setRetention(null);
      history.reload();
    } catch (reason) {
      setModalError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };
  const deleteAll = async () => {
    if (running.current || confirmation !== "DELETE ALL LOCAL HISTORY") return;
    running.current = true;
    setModalError(null);
    setBusy(true);
    try {
      const receipt = await commands.deleteAll(confirmation);
      const next = `Deleted ${receipt.sessions} local ${receipt.sessions === 1 ? "session" : "sessions"}. Existing backups were not changed.`;
      setMessage(next);
      announce(next);
      setDeleteAllOpen(false);
      setConfirmation("");
      history.reload();
    } catch (reason) {
      setModalError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <PageHeader title="Data controls" />
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

      <div className="data-stack">
        <section className="panel" aria-labelledby="retention-heading">
          <div className="panel__header">
            <div>
              <h2 id="retention-heading">Data retention</h2>
            </div>
            <CalendarClock aria-hidden="true" />
          </div>
          {settings.state === "error" ? (
            <p className="form-error" role="alert">
              {settings.error.message}
            </p>
          ) : null}
          <div className="control-row">
            <label className="inline-field">
              <span>Keep sessions for</span>
              <select
                aria-label="Keep sessions for"
                disabled={settings.data === null || policyBusy}
                value={days ?? "forever"}
                onChange={(event) =>
                  setRetentionChoice(
                    event.currentTarget.value === "forever"
                      ? null
                      : Number(event.currentTarget.value),
                  )
                }
              >
                <option value="forever">Until explicitly deleted</option>
                <option value={7}>7 days</option>
                <option value={30}>30 days</option>
                <option value={90}>90 days</option>
                <option value={365}>1 year</option>
                {days !== null && ![7, 30, 90, 365].includes(days) ? (
                  <option value={days}>{days} days (current)</option>
                ) : null}
              </select>
            </label>
            <Button
              variant="primary"
              disabled={settings.data === null || policyBusy}
              onClick={() => void saveRetentionPolicy()}
            >
              {policyBusy ? "Saving…" : "Save"}
            </Button>
            {days === null ? null : (
              <Button
                disabled={policyBusy}
                onClick={() => void previewRetention()}
              >
                {`Preview ${days}-day cleanup`}
              </Button>
            )}
          </div>
        </section>

        <BackupControls
          onRestored={() => {
            previewGeneration.current += 1;
            setModalError(null);
            setMessage(null);
            setError(null);
            setRetention(null);
            setDeleteAllOpen(false);
            setConfirmation("");
            history.reload();
          }}
        />

        <section className="danger-zone" aria-labelledby="delete-all-heading">
          <div>
            <h2 id="delete-all-heading">Delete all local history</h2>
            <p>
              {data.total} {data.total === 1 ? "session" : "sessions"} stored.
              Requires a typed confirmation.
            </p>
          </div>
          <Button
            variant="danger"
            disabled={data.total === 0}
            onClick={() => {
              setModalError(null);
              setDeleteAllOpen(true);
            }}
          >
            <Database aria-hidden="true" /> Delete all…
          </Button>
        </section>
      </div>

      {retention === null ? null : (
        <Modal
          title={`Preview ${days}-day retention policy`}
          description="This is an immutable selection preview. Nothing has been deleted yet."
          onClose={closeRetention}
          closeDisabled={busy}
          size="wide"
          footer={
            <>
              <Button disabled={busy} onClick={closeRetention}>
                Cancel
              </Button>
              <Button
                variant="danger"
                disabled={typeof retention === "string" || busy}
                onClick={() => void applyRetention()}
              >
                {busy ? "Applying…" : "Apply exactly this preview"}
              </Button>
            </>
          }
        >
          {modalError === null ? null : (
            <p className="form-error" role="alert">
              {modalError}
            </p>
          )}
          {retention === "loading" ? (
            <LoadingState label="Computing exact retention scope" />
          ) : retention === "failed" ? null : (
            <>
              <DefinitionList
                rows={[
                  { term: "Cutoff", value: retention.cutoff },
                  { term: "Sessions", value: retention.sessionIds.length },
                  {
                    term: "Records",
                    value: retention.rawObservations,
                  },
                  { term: "Messages", value: retention.messages },
                  { term: "Summaries", value: retention.summaries },
                ]}
              />
              {retention.sessionTitles.length === 0 ? (
                <p>No sessions are older than this cutoff.</p>
              ) : (
                <section>
                  <h3>Exact sessions selected</h3>
                  <ul>
                    {retention.sessionTitles.map((title) => (
                      <li key={title}>{title}</li>
                    ))}
                  </ul>
                </section>
              )}
              <Disclosure>{retention.disclosure}</Disclosure>
            </>
          )}
        </Modal>
      )}

      {deleteAllOpen ? (
        <Modal
          title="Delete all local history?"
          description="This removes every current session and linked local record."
          onClose={closeDeleteAll}
          closeDisabled={busy}
          footer={
            <>
              <Button disabled={busy} onClick={closeDeleteAll}>
                Cancel — keep all history
              </Button>
              <Button
                variant="danger"
                disabled={confirmation !== "DELETE ALL LOCAL HISTORY" || busy}
                onClick={() => void deleteAll()}
              >
                Delete all local history
              </Button>
            </>
          }
        >
          {modalError === null ? null : (
            <p className="form-error" role="alert">
              {modalError}
            </p>
          )}
          <Disclosure>
            Existing backups, WAL history, filesystem snapshots, and SSD blocks
            are not securely erased.
          </Disclosure>
          <label className="field-stack">
            <span>
              Type <code>DELETE ALL LOCAL HISTORY</code> to confirm
            </span>
            <input
              value={confirmation}
              disabled={busy}
              onChange={(event) => setConfirmation(event.currentTarget.value)}
              autoComplete="off"
            />
          </label>
        </Modal>
      ) : null}
    </div>
  );
}

import { ArchiveRestore, CalendarClock, Database, Trash2 } from "lucide-react";
import { useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  DeletionPreview,
  RetentionPreview,
  SessionFilters,
} from "../contracts.js";
import { useAnnounce } from "../components/Announcer.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  EmptyState,
  ErrorState,
  LoadingState,
  Modal,
  PageHeader,
  SuccessMessage,
} from "../components/Primitives.js";

const ALL_SESSIONS: SessionFilters = {
  text: "",
  harness: "all",
  project: "",
  branch: "",
  dateRange: "all",
  tool: "",
  skill: "",
  agent: "",
};

export function DataControlsPage() {
  const commands = useCommands();
  const history = useCommandResource(
    () => commands.searchSessions(ALL_SESSIONS),
    "data-history",
  );
  const [days, setDays] = useState(30);
  const [retention, setRetention] = useState<
    RetentionPreview | "loading" | null
  >(null);
  const [single, setSingle] = useState<DeletionPreview | "loading" | null>(
    null,
  );
  const [deleteAllOpen, setDeleteAllOpen] = useState(false);
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const announce = useAnnounce();

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

  const previewRetention = async () => {
    setRetention("loading");
    setError(null);
    try {
      setRetention(await commands.previewRetention(days));
    } catch (reason) {
      setRetention(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const applyRetention = async () => {
    if (retention === null || retention === "loading") return;
    setBusy(true);
    try {
      const receipt = await commands.applyRetention(retention.previewToken);
      await commands.patchSettings({ retention_days: retention.retentionDays });
      const next = `Retention applied exactly as previewed: ${receipt.sessions} sessions and ${receipt.ftsRows} search rows deleted.`;
      setMessage(next);
      announce(next);
      setRetention(null);
      history.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  const previewSingle = async (sessionId: string) => {
    setSingle("loading");
    setError(null);
    try {
      setSingle(await commands.previewSessionDeletion(sessionId));
    } catch (reason) {
      setSingle(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const deleteSingle = async () => {
    if (single === null || single === "loading") return;
    setBusy(true);
    try {
      const sessionId = single.sessionIds[0];
      if (sessionId === undefined)
        throw new Error("Deletion preview contains no session.");
      const receipt = await commands.deleteSession(
        sessionId,
        single.previewToken,
      );
      const next = `Deleted ${single.sessionTitles[0] ?? sessionId}; ${receipt.sessions} session removed.`;
      setMessage(next);
      announce(next);
      setSingle(null);
      history.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  const deleteAll = async () => {
    setBusy(true);
    try {
      const receipt = await commands.deleteAll(confirmation);
      const next = `Deleted ${receipt.sessions} local sessions. Existing backups were not changed.`;
      setMessage(next);
      announce(next);
      setDeleteAllOpen(false);
      setConfirmation("");
      history.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <PageHeader
        eyebrow="Preview before mutation"
        title="Data controls"
        description="History stays until you delete it. Retention, one-session deletion, and delete-all operate on exact previews across raw evidence, projections, search rows, and summaries."
      />
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {message === null ? null : <SuccessMessage>{message}</SuccessMessage>}

      <div className="data-control-grid">
        <section className="panel" aria-labelledby="retention-heading">
          <div className="panel__header">
            <div>
              <p className="eyebrow">Policy preview</p>
              <h2 id="retention-heading">Data retention</h2>
            </div>
            <CalendarClock aria-hidden="true" />
          </div>
          <p>
            Default: keep until explicitly deleted. A policy preview never
            changes history.
          </p>
          <label className="field-stack">
            <span>Keep sessions for</span>
            <select
              value={days}
              onChange={(event) => setDays(Number(event.currentTarget.value))}
            >
              <option value={7}>7 days</option>
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
              <option value={365}>1 year</option>
            </select>
          </label>
          <Button variant="primary" onClick={() => void previewRetention()}>
            Preview {days}-day policy
          </Button>
        </section>

        <section className="panel" aria-labelledby="backup-heading">
          <div className="panel__header">
            <div>
              <p className="eyebrow">Deletion boundary</p>
              <h2 id="backup-heading">Backups and physical media</h2>
            </div>
            <ArchiveRestore aria-hidden="true" />
          </div>
          <p>
            Deletion removes selected database rows and search projections
            transactionally. Existing backups, filesystem snapshots, WAL
            remnants, and SSD flash blocks are not rewritten.
          </p>
          <Disclosure>
            Cutokyo never promises physical secure erasure. Checkpoint and
            vacuum can reduce ordinary remnants, but full-disk encryption and
            backup lifecycle controls define the stronger boundary.
          </Disclosure>
        </section>
      </div>

      <section className="panel" aria-labelledby="single-heading">
        <div className="panel__header">
          <div>
            <p className="eyebrow">Narrow deletion</p>
            <h2 id="single-heading">Delete one session</h2>
          </div>
          <span className="quiet-label">{data.total} stored</span>
        </div>
        {data.sessions.length === 0 ? (
          <EmptyState
            compact
            title="No sessions to delete"
            description="Local history is empty. No destructive action is available."
          />
        ) : (
          <div className="deletion-session-list">
            {data.sessions.map((session) => (
              <article key={session.id}>
                <div>
                  <strong>{session.title ?? session.id}</strong>
                  <span>
                    {session.id} · {session.project ?? "Project unknown"}
                  </span>
                </div>
                <Button
                  variant="danger"
                  size="small"
                  onClick={() => void previewSingle(session.id)}
                >
                  <Trash2 aria-hidden="true" /> Delete only this
                </Button>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="danger-zone" aria-labelledby="delete-all-heading">
        <div>
          <p className="eyebrow">Destructive action</p>
          <h2 id="delete-all-heading">Delete all local history</h2>
          <p>
            Requires the exact confirmation phrase. Cancelling preserves every
            session.
          </p>
        </div>
        <Button
          variant="danger"
          disabled={data.sessions.length === 0}
          onClick={() => setDeleteAllOpen(true)}
        >
          <Database aria-hidden="true" /> Delete all…
        </Button>
      </section>

      {retention === null ? null : (
        <Modal
          title={`Preview ${days}-day retention policy`}
          description="This is an immutable selection preview. Nothing has been deleted yet."
          onClose={() => setRetention(null)}
          size="wide"
          footer={
            <>
              <Button onClick={() => setRetention(null)}>Cancel</Button>
              <Button
                variant="danger"
                disabled={retention === "loading" || busy}
                onClick={() => void applyRetention()}
              >
                {busy ? "Applying…" : "Apply exactly this preview"}
              </Button>
            </>
          }
        >
          {retention === "loading" ? (
            <LoadingState label="Computing exact retention scope" />
          ) : (
            <>
              <DefinitionList
                rows={[
                  { term: "Cutoff", value: retention.cutoff },
                  { term: "Sessions", value: retention.sessionIds.length },
                  {
                    term: "Raw observations",
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

      {single === null ? null : (
        <Modal
          title="Delete exactly one session?"
          description="Neighboring sessions remain untouched."
          onClose={() => setSingle(null)}
          footer={
            <>
              <Button onClick={() => setSingle(null)}>Cancel</Button>
              <Button
                variant="danger"
                disabled={single === "loading" || busy}
                onClick={() => void deleteSingle()}
              >
                Confirm one-session deletion
              </Button>
            </>
          }
        >
          {single === "loading" ? (
            <LoadingState label="Counting linked rows" />
          ) : (
            <>
              <DefinitionList
                rows={[
                  {
                    term: "Selected session",
                    value: single.sessionTitles.join(", "),
                  },
                  { term: "Raw observations", value: single.rawObservations },
                  { term: "Messages", value: single.messages },
                  { term: "Search rows", value: single.ftsRows },
                ]}
              />
              <Disclosure>{single.disclosure}</Disclosure>
            </>
          )}
        </Modal>
      )}

      {deleteAllOpen ? (
        <Modal
          title="Delete all local history?"
          description="This removes every current session and linked local record."
          onClose={() => {
            setDeleteAllOpen(false);
            setConfirmation("");
          }}
          footer={
            <>
              <Button
                onClick={() => {
                  setDeleteAllOpen(false);
                  setConfirmation("");
                }}
              >
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
              onChange={(event) => setConfirmation(event.currentTarget.value)}
              autoComplete="off"
            />
          </label>
        </Modal>
      ) : null}
    </div>
  );
}

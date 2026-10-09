import { ArchiveRestore } from "lucide-react";
import { useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  BackupInfo,
  BackupRestorePreview,
  BackupRestoreReceipt,
} from "../contracts.js";
import { useAnnounce } from "./Announcer.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  LoadingState,
  Modal,
  SuccessMessage,
  formatBytes,
} from "./Primitives.js";

const errorText = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);
const size = (bytes: number) =>
  `${formatBytes(bytes)} (${bytes.toLocaleString("en-US")} bytes)`;
const date = (value: string) => new Date(value).toLocaleString();

function BackupDetails({ backup }: { readonly backup: BackupInfo }) {
  return (
    <DefinitionList
      rows={[
        { term: "Location", value: backup.path },
        { term: "Created", value: date(backup.createdAt) },
        { term: "Size", value: size(backup.byteLength) },
        { term: "Sessions", value: backup.sessionCount },
      ]}
    />
  );
}

export function BackupControls({
  onRestored,
}: {
  readonly onRestored: () => void;
}) {
  const commands = useCommands();
  const announce = useAnnounce();
  const backups = useCommandResource(
    () => commands.listBackups(),
    "local-backups",
  );
  const [modal, setModal] = useState<"create" | "restore" | null>(null);
  const [destination, setDestination] = useState("");
  const [otherPath, setOtherPath] = useState("");
  const [selected, setSelected] = useState<BackupInfo | null>(null);
  const [preview, setPreview] = useState<BackupRestorePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [created, setCreated] = useState<BackupInfo | null>(null);
  const [restored, setRestored] = useState<BackupRestoreReceipt | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const running = useRef(false);
  const selection = useRef(0);

  const close = () => {
    if (running.current) return;
    selection.current += 1;
    setModal(null);
    setError(null);
  };
  const openCreate = () => {
    setCreated(null);
    setError(null);
    setModal("create");
  };
  const reviewRestore = async (path: string, backup: BackupInfo | null) => {
    const revision = ++selection.current;
    setSelected(backup);
    setPreview(null);
    setRestored(null);
    setError(null);
    setPreviewLoading(true);
    setModal("restore");
    try {
      const next = await commands.previewBackupRestore(path);
      if (revision !== selection.current) return;
      setSelected(next.backup);
      setPreview(next);
    } catch (reason) {
      if (revision === selection.current) setError(errorText(reason));
    } finally {
      if (revision === selection.current) setPreviewLoading(false);
    }
  };
  const create = async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError(null);
    try {
      const backup = await commands.createBackup(
        destination.trim() || undefined,
      );
      setCreated(backup);
      backups.reload();
      announce(`Backup created at ${backup.path}.`);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };
  const restore = async () => {
    if (running.current || preview === null) return;
    running.current = true;
    setBusy(true);
    setError(null);
    try {
      const receipt = await commands.restoreBackup(preview.previewToken);
      setRestored(receipt);
      onRestored();
      backups.reload();
      announce(
        `Restored ${receipt.restoredSessionCount} ${receipt.restoredSessionCount === 1 ? "session" : "sessions"}. Previous history retained at ${receipt.recoveryPath}.`,
      );
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };

  return (
    <section className="panel" aria-labelledby="backup-heading">
      <div className="panel__header">
        <div>
          <h2 id="backup-heading">Backups</h2>
        </div>
        <ArchiveRestore aria-hidden="true" />
      </div>
      <div className="control-row">
        <Button variant="primary" onClick={openCreate}>
          Create backup
        </Button>
      </div>
      {backups.state === "loading" && backups.data === null ? (
        <LoadingState label="Finding local backups" />
      ) : null}
      {backups.state === "error" ? (
        <div>
          <p className="form-error" role="alert">
            {backups.error.message}
          </p>
          <Button onClick={backups.reload}>Retry backup list</Button>
        </div>
      ) : null}
      {backups.data?.length === 0 ? <p>No backups yet.</p> : null}
      <div className="deletion-session-list">
        {backups.data?.map((backup) => (
          <article key={backup.path}>
            <div>
              <strong>{date(backup.createdAt)}</strong>
              <span>
                {backup.sessionCount}{" "}
                {backup.sessionCount === 1 ? "session" : "sessions"} ·{" "}
                {size(backup.byteLength)}
              </span>
              <span>{backup.path}</span>
            </div>
            <Button
              size="small"
              onClick={() => void reviewRestore(backup.path, backup)}
            >
              Restore…
            </Button>
          </article>
        ))}
      </div>
      <details className="advanced">
        <summary>Advanced</summary>
        <label className="field-stack">
          <span>Restore a backup from another location</span>
          <input
            value={otherPath}
            onChange={(event) => setOtherPath(event.currentTarget.value)}
            placeholder="Full path to a Cutokyo backup directory"
          />
        </label>
        <div className="control-row">
          <Button
            disabled={otherPath.trim() === ""}
            onClick={() => void reviewRestore(otherPath.trim(), null)}
          >
            Review backup
          </Button>
        </div>
        <Disclosure>
          Backups are owner-only local files, not application-encrypted. Use
          full-disk encryption. Deletion does not rewrite existing backups,
          filesystem snapshots, WAL remnants, or SSD blocks; Cutokyo does not
          promise physical secure erasure.
        </Disclosure>
      </details>

      {modal === "create" ? (
        <Modal
          title={created === null ? "Create a local backup" : "Backup created"}
          closeDisabled={busy}
          onClose={close}
          footer={
            created === null ? (
              <>
                <Button disabled={busy} onClick={close}>
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() => void create()}
                >
                  {busy ? "Creating backup…" : "Create backup"}
                </Button>
              </>
            ) : (
              <Button onClick={close}>Done</Button>
            )
          }
        >
          {error === null ? null : (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          {created === null ? (
            <>
              <p>
                Default: a private, timestamped folder in your Cutokyo data
                directory’s <code>backups</code> folder.
              </p>
              <details>
                <summary>Use another location (optional)</summary>
                <label className="field-stack">
                  <span>New backup directory</span>
                  <input
                    disabled={busy}
                    value={destination}
                    onChange={(event) =>
                      setDestination(event.currentTarget.value)
                    }
                    placeholder="Leave blank for the default location"
                  />
                </label>
                <p>
                  Enter a new directory path. Existing locations are never
                  overwritten.
                </p>
              </details>
              {busy ? (
                <LoadingState label="Creating and verifying the recovery copy" />
              ) : null}
            </>
          ) : (
            <>
              <SuccessMessage>Your local backup is ready.</SuccessMessage>
              <BackupDetails backup={created} />
            </>
          )}
        </Modal>
      ) : null}

      {modal === "restore" ? (
        <Modal
          title={
            restored === null ? "Restore this backup?" : "History restored"
          }
          closeDisabled={busy}
          onClose={close}
          footer={
            restored === null ? (
              <>
                <Button disabled={busy} onClick={close}>
                  Cancel — keep current history
                </Button>
                <Button
                  variant="danger"
                  disabled={busy || previewLoading || preview === null}
                  onClick={() => void restore()}
                >
                  {busy ? "Restoring history…" : "Restore selected backup"}
                </Button>
              </>
            ) : (
              <Button onClick={close}>Done</Button>
            )
          }
        >
          {error === null ? null : (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          {previewLoading ? (
            <LoadingState label="Verifying backup and current history" />
          ) : null}
          {selected === null ? null : <BackupDetails backup={selected} />}
          {restored === null ? (
            <>
              {preview === null ? null : (
                <p>
                  {preview.currentSessionCount} current{" "}
                  {preview.currentSessionCount === 1 ? "session" : "sessions"}{" "}
                  will be replaced by {preview.backup.sessionCount} backed-up{" "}
                  {preview.backup.sessionCount === 1 ? "session" : "sessions"}.
                  The previous history will remain in a recovery copy.
                </p>
              )}

              {busy ? (
                <LoadingState label="Replacing history and checking restored integrity" />
              ) : null}
            </>
          ) : (
            <>
              <SuccessMessage>
                Restored {restored.restoredSessionCount}{" "}
                {restored.restoredSessionCount === 1 ? "session" : "sessions"}.
              </SuccessMessage>
              <DefinitionList
                rows={[
                  { term: "Recovery copy", value: restored.recoveryPath },
                  { term: "Integrity result", value: restored.integrityResult },
                ]}
              />
            </>
          )}
        </Modal>
      ) : null}
    </section>
  );
}

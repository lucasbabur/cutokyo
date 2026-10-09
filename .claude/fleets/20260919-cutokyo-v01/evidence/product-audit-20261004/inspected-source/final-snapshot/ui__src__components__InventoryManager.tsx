import { Copy, FileText, Save, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { ActionReceipt, Harness, InventoryItem } from "../contracts.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  ErrorState,
  LoadingState,
  Modal,
} from "./Primitives.js";

const HARNESS_NAMES: Record<Harness, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
};

type Mode = "edit" | "install" | "remove" | "discard";

export function InventoryManager({
  item,
  onClose,
  onComplete,
}: {
  readonly item: InventoryItem;
  readonly onClose: () => void;
  readonly onComplete: (message: string) => void;
}) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getInventoryDocument(item.id),
    item.id,
  );
  const [content, setContent] = useState("");
  const [mode, setMode] = useState<Mode>("edit");
  const [target, setTarget] = useState<Harness | "">("");
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (resource.state !== "ready") return;
    setContent(resource.data.content);
    setTarget(
      resource.data.installTargets.find((entry) => entry.available)?.harness ??
        "",
    );
  }, [resource.state, resource.data]);

  const document = resource.data;
  const dirty = document !== null && content !== document.content;
  const destination = document?.installTargets.find(
    (entry) => entry.harness === target,
  );
  const close = () => {
    if (inFlight.current) return;
    if (dirty && mode !== "discard") setMode("discard");
    else onClose();
  };
  const mutate = async (action: () => Promise<ActionReceipt>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      const receipt = await action();
      if (!receipt.ok || receipt.status !== "success") {
        throw new Error(receipt.message);
      }
      onComplete(receipt.message);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const changeMode = (next: Mode) => {
    setMode(next);
    setError(null);
  };

  return (
    <Modal
      title={
        mode === "remove"
          ? `Remove ${item.name}?`
          : mode === "discard"
            ? "Discard unsaved changes?"
            : mode === "install"
              ? `Install ${item.name} in another harness`
              : `Manage ${item.name}`
      }
      eyebrow={mode === "edit" ? "Agent tools" : "Confirm change"}
      description={
        mode === "remove"
          ? "Only this installation will be removed. Copies in other harnesses are unchanged."
          : mode === "discard"
            ? "Your edits have not been saved. Keep editing or discard them."
            : "Changes apply to the native installation shown below. Nothing is sent to a provider."
      }
      size="wide"
      onClose={close}
      closeDisabled={busy}
      footer={
        mode === "discard" ? (
          <>
            <Button onClick={() => changeMode("edit")}>Keep editing</Button>
            <Button variant="danger" onClick={onClose}>
              Discard changes
            </Button>
          </>
        ) : mode === "remove" ? (
          <>
            <Button disabled={busy} onClick={() => changeMode("edit")}>
              Cancel
            </Button>
            <Button
              variant="danger"
              disabled={busy || document === null || !document.removable}
              onClick={() => {
                if (document === null) return;
                void mutate(() =>
                  commands.removeInventoryItem(item.id, document.revision),
                );
              }}
            >
              <Trash2 aria-hidden="true" />
              {busy ? "Removing…" : "Remove installation"}
            </Button>
          </>
        ) : mode === "install" ? (
          <>
            <Button disabled={busy} onClick={() => changeMode("edit")}>
              Back
            </Button>
            <Button
              variant="primary"
              disabled={busy || document === null || !destination?.available}
              onClick={() => {
                if (document === null || target === "") return;
                void mutate(() =>
                  commands.installInventoryItem(
                    item.id,
                    document.revision,
                    target,
                  ),
                );
              }}
            >
              <Copy aria-hidden="true" />
              {busy
                ? "Installing…"
                : `Install in ${target === "" ? "selected harness" : HARNESS_NAMES[target]}`}
            </Button>
          </>
        ) : (
          <>
            <Button disabled={busy} onClick={close}>
              {dirty ? "Cancel" : "Done"}
            </Button>
            {document?.editable ? (
              <Button
                variant="primary"
                disabled={busy || !dirty}
                onClick={() => {
                  void mutate(() =>
                    commands.saveInventoryDocument(
                      item.id,
                      document.revision,
                      content,
                    ),
                  );
                }}
              >
                <Save aria-hidden="true" />
                {busy ? "Saving…" : "Save changes"}
              </Button>
            ) : null}
          </>
        )
      }
    >
      {error === null ? null : (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {resource.state === "loading" && document === null ? (
        <LoadingState label="Reading native installation" />
      ) : resource.state === "error" ? (
        <ErrorState
          title="Installation could not be read"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : document === null || mode === "discard" ? null : (
        <div className="inventory-manager">
          <DefinitionList
            rows={[
              { term: "Installation", value: item.name },
              {
                term: "Harness",
                value: item.harnesses
                  .map((harness) => HARNESS_NAMES[harness])
                  .join(", "),
              },
              { term: "Scope", value: item.scope },
              { term: "Source", value: <code>{item.origin}</code> },
            ]}
          />
          {mode === "remove" ? (
            <Disclosure>
              Removing this {item.kind} changes its native configuration. A
              recovery copy is retained. Session history is not deleted.
            </Disclosure>
          ) : mode === "install" ? (
            <>
              <label className="field-stack">
                <span>Destination harness</span>
                <select
                  value={target}
                  disabled={busy}
                  onChange={(event) =>
                    setTarget(event.currentTarget.value as Harness | "")
                  }
                >
                  <option value="">Choose a harness</option>
                  {document.installTargets.map((entry) => (
                    <option
                      key={entry.harness}
                      value={entry.harness}
                      disabled={!entry.available}
                    >
                      {HARNESS_NAMES[entry.harness]}
                      {entry.available ? "" : " · unavailable"}
                    </option>
                  ))}
                </select>
              </label>
              {destination === undefined ? null : (
                <div className="installation-target">
                  <strong>Will be installed at</strong>
                  <code>{destination.destination}</code>
                </div>
              )}
              <ul className="installation-options">
                {document.installTargets.map((entry) => (
                  <li key={entry.harness}>
                    <strong>{HARNESS_NAMES[entry.harness]}</strong>
                    <span>
                      {entry.available
                        ? entry.destination
                        : (entry.reason ?? "This format is not supported.")}
                    </span>
                  </li>
                ))}
              </ul>
              <Disclosure>
                This creates an independent copy. Later edits do not sync
                between harnesses. Existing installations are never overwritten.
                {item.kind === "skill"
                  ? " Supporting files are copied with the skill."
                  : ""}
              </Disclosure>
            </>
          ) : (
            <>
              <div className="inventory-manager__actions">
                <Button
                  size="small"
                  disabled={
                    busy ||
                    dirty ||
                    !document.installTargets.some((entry) => entry.available)
                  }
                  onClick={() => changeMode("install")}
                >
                  <Copy aria-hidden="true" /> Install in another harness
                </Button>
                {document.removable ? (
                  <Button
                    size="small"
                    variant="danger"
                    disabled={busy || dirty}
                    onClick={() => changeMode("remove")}
                  >
                    <Trash2 aria-hidden="true" /> Remove
                  </Button>
                ) : null}
              </div>
              {dirty ? (
                <p className="quiet-label" role="status">
                  Unsaved changes. Save before installing or removing this item.
                </p>
              ) : null}
              {document.unavailableReason === null ? null : (
                <Disclosure>{document.unavailableReason}</Disclosure>
              )}
              <label className="field-stack">
                <span>
                  <FileText aria-hidden="true" />
                  {document.editable
                    ? "Source content"
                    : "Installation details"}
                  <span className="quiet-label">{document.format}</span>
                </span>
                <textarea
                  className="source-editor"
                  aria-label="Source content"
                  value={content}
                  readOnly={!document.editable}
                  disabled={busy}
                  spellCheck={false}
                  rows={12}
                  onChange={(event) => setContent(event.currentTarget.value)}
                />
              </label>
              {document.editable ? (
                <p className="quiet-label">
                  Edit this installation's content. Cutokyo validates it before
                  saving and keeps a recovery copy. Credentials stay local.
                </p>
              ) : null}
              {document.installTargets.some(
                (entry) => entry.available,
              ) ? null : (
                <p className="quiet-label">
                  {document.installTargets
                    .map(
                      (entry) =>
                        `${HARNESS_NAMES[entry.harness]}: ${entry.reason ?? "not supported"}`,
                    )
                    .join(". ")}
                </p>
              )}
            </>
          )}
        </div>
      )}
    </Modal>
  );
}

import {
  AlertTriangle,
  Copy,
  Link2,
  Plus,
  Save,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { ActionReceipt, Harness, InventoryItem } from "../contracts.js";
import { routeHref } from "../router.js";
import { HARNESS_NAMES, HARNESS_ORDER, HarnessMark } from "./HarnessMark.js";
import { KIND_LABEL, KindIcon } from "./KindIcon.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  ErrorState,
  LoadingState,
  Modal,
  ProvenanceDetails,
  StatusPill,
} from "./Primitives.js";

type Mode = "edit" | "install" | "remove" | "discard";
type Tab = "file" | "details";
const TABS: readonly { readonly id: Tab; readonly label: string }[] = [
  { id: "file", label: "File" },
  { id: "details", label: "Details" },
];

/** A harness cell in the tool list asked for a copy; `nonce` makes repeats count. */
export interface InstallRequest {
  readonly harness: Harness;
  readonly nonce: number;
}

/**
 * Side panel for one tool: where it is installed, its source file, and the
 * install, save and remove actions. Confirmations open as dialogs above it.
 */
export function InventoryManager({
  item,
  installations,
  installRequest,
  onSelect,
  onClose,
  onComplete,
}: {
  readonly item: InventoryItem;
  /** Every installation of this tool; `item` is the one being shown. */
  readonly installations: readonly InventoryItem[];
  readonly installRequest: InstallRequest | null;
  readonly onSelect: (item: InventoryItem) => void;
  readonly onClose: () => void;
  readonly onComplete: (message: string) => void;
}) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getInventoryDocument(item.id),
    item.id,
  );
  const [draft, setDraft] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("edit");
  const [target, setTarget] = useState<Harness | "">("");
  const [acknowledgedDrops, setAcknowledgedDrops] = useState(false);
  const [tab, setTab] = useState<Tab>("file");
  const [busy, setBusy] = useState(false);
  const [leave, setLeave] = useState<(() => void) | null>(null);
  const inFlight = useRef(false);
  const handledRequest = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const document = resource.data;
  const content = draft ?? document?.content ?? "";
  const dirty = document !== null && content !== document.content;
  const managedCaptureHook =
    item.kind === "hook" &&
    item.managedByCutokyo &&
    document?.editable === false &&
    document.removable === false;
  const destination = document?.installTargets.find(
    (entry) => entry.harness === target,
  );
  const dropped = destination?.droppedFields ?? [];
  const installBlocked = dropped.length > 0 && !acknowledgedDrops;

  const changeMode = (next: Mode) => {
    setAcknowledgedDrops(false);
    setMode(next);
    setError(null);
  };
  const startInstall = (harness: Harness) => {
    if (dirty) {
      setError("Save or revert your changes before installing a copy.");
      return;
    }
    setTarget(harness);
    changeMode("install");
  };
  /** Runs `action` now, or after the user agrees to drop unsaved edits. */
  const guard = (action: () => void) => {
    if (inFlight.current) return;
    if (dirty) {
      setLeave(() => action);
      setMode("discard");
    } else action();
  };

  useEffect(() => {
    if (
      installRequest === null ||
      document === null ||
      handledRequest.current === installRequest.nonce
    ) {
      return;
    }
    handledRequest.current = installRequest.nonce;
    const entry = document.installTargets.find(
      (candidate) => candidate.harness === installRequest.harness,
    );
    if (entry?.available) startInstall(installRequest.harness);
    else {
      setError(
        entry?.reason ??
          `${HARNESS_NAMES[installRequest.harness]} cannot receive a copy.`,
      );
    }
    // startInstall reads the latest render; only a new request should rerun this.
  }, [installRequest, document]);

  const mutate = async (
    action: () => Promise<ActionReceipt>,
    reloadDocument = true,
  ) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      const receipt = await action();
      if (!receipt.ok || receipt.status !== "success") {
        throw new Error(receipt.message);
      }
      setDraft(null);
      setMode("edit");
      onComplete(receipt.message);
      if (reloadDocument) resource.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  const unusual =
    item.state === "enabled" ||
    item.state === "installed" ||
    item.state === "configured"
      ? null
      : item.state;

  return (
    <aside
      className="tool-panel"
      aria-label={item.name}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || mode !== "edit") return;
        event.preventDefault();
        guard(onClose);
      }}
    >
      <header className="tool-panel__header">
        <span className={`tool-row__icon tool-row__icon--${item.kind}`}>
          <KindIcon kind={item.kind} />
        </span>
        <div className="tool-panel__title">
          <h2>
            {item.name}
            {unusual === null ? null : (
              <span className={`quiet-badge quiet-badge--${unusual}`}>
                {unusual}
              </span>
            )}
          </h2>
          {item.description === "" ? null : <p>{item.description}</p>}
        </div>
        <Button
          variant="quiet"
          size="small"
          aria-label="Close details"
          disabled={busy}
          onClick={() => guard(onClose)}
        >
          <X aria-hidden="true" />
        </Button>
      </header>

      <ul className="tool-panel__where" aria-label="Harnesses">
        {HARNESS_ORDER.map((harness) => {
          const owner = installations.find((entry) =>
            entry.harnesses.includes(harness),
          );
          const installTarget = document?.installTargets.find(
            (entry) => entry.harness === harness,
          );
          const name = (
            <span className="tool-panel__harness">
              <HarnessMark harness={harness} />
              {HARNESS_NAMES[harness]}
            </span>
          );
          if (owner !== undefined) {
            const current = owner.id === item.id;
            return (
              <li key={harness}>
                {installations.length > 1 && !current ? (
                  <button
                    type="button"
                    className="tool-panel__where-row"
                    disabled={busy}
                    onClick={() => guard(() => onSelect(owner))}
                  >
                    {name}
                    <code title={owner.origin}>
                      <bdi>{owner.origin}</bdi>
                    </code>
                  </button>
                ) : (
                  <div
                    className="tool-panel__where-row is-current"
                    aria-current={installations.length > 1 ? "true" : undefined}
                  >
                    {name}
                    <code title={owner.origin}>
                      <bdi>{owner.origin}</bdi>
                    </code>
                  </div>
                )}
              </li>
            );
          }
          return (
            <li key={harness}>
              <div className="tool-panel__where-row is-missing">
                {name}
                {installTarget ===
                undefined ? null : installTarget.available ? (
                  <Button
                    size="small"
                    aria-label={`Install to ${HARNESS_NAMES[harness]}`}
                    disabled={busy}
                    onClick={() => startInstall(harness)}
                  >
                    <Plus aria-hidden="true" /> Install
                  </Button>
                ) : (
                  <span
                    className="tool-panel__unavailable"
                    title={installTarget.reason ?? undefined}
                  >
                    Not available
                    <span className="sr-only">: {installTarget.reason}</span>
                  </span>
                )}
              </div>
            </li>
          );
        })}
      </ul>

      {error === null ? null : (
        <p className="form-error tool-panel__error" role="alert">
          {error}
        </p>
      )}

      <div
        className="tool-panel__tabs"
        role="tablist"
        aria-label="Tool sections"
      >
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            type="button"
            role="tab"
            id={`tool-tab-${id}`}
            aria-selected={tab === id}
            aria-controls="tool-panel-body"
            tabIndex={tab === id ? 0 : -1}
            className={tab === id ? "tab is-active" : "tab"}
            onClick={() => setTab(id)}
            onKeyDown={(event) => {
              if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
                return;
              }
              event.preventDefault();
              const next = TABS[(TABS.findIndex((t) => t.id === tab) + 1) % 2]!;
              setTab(next.id);
              globalThis.document
                .getElementById(`tool-tab-${next.id}`)
                ?.focus();
            }}
          >
            {label}
          </button>
        ))}
      </div>

      <div
        id="tool-panel-body"
        role="tabpanel"
        aria-labelledby={`tool-tab-${tab}`}
        className="tool-panel__body"
      >
        {resource.state === "loading" && document === null ? (
          <LoadingState label="Loading" />
        ) : resource.state === "error" ? (
          <ErrorState
            title="Installation could not be read"
            error={resource.error}
            onRetry={resource.reload}
          />
        ) : document === null ? null : tab === "file" ? (
          <>
            {document.unavailableReason === null ? null : (
              <Disclosure>{document.unavailableReason}</Disclosure>
            )}
            <textarea
              className="source-editor tool-panel__editor"
              aria-label="Source content"
              value={content}
              readOnly={!document.editable}
              disabled={busy}
              spellCheck={false}
              onChange={(event) => setDraft(event.currentTarget.value)}
            />
          </>
        ) : (
          <div className="tool-panel__details">
            <DefinitionList
              rows={[
                { term: "Kind", value: KIND_LABEL[item.kind] },
                { term: "Scope", value: item.scope },
                { term: "Format", value: document.format },
                { term: "Source", value: <code>{item.origin}</code> },
                ...(item.linkTarget == null
                  ? []
                  : [
                      {
                        term: "Linked to",
                        value: (
                          <>
                            <Link2 aria-hidden="true" />{" "}
                            <code>{item.linkTarget}</code>
                          </>
                        ),
                      },
                    ]),
              ]}
            />
            {(document.notes ?? []).length === 0 ? null : (
              <ul className="note-list">
                {document.notes?.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            )}
            {document.installTargets.some(
              (entry) => (entry.conversions?.length ?? 0) > 0,
            ) ? (
              <section className="manage-section">
                <h3>Conversions when installing</h3>
                <ul className="note-list">
                  {document.installTargets.flatMap((entry) =>
                    (entry.conversions ?? []).map((line) => (
                      <li key={`${entry.harness}-${line}`}>
                        {HARNESS_NAMES[entry.harness]}: {line}
                      </li>
                    )),
                  )}
                </ul>
              </section>
            ) : null}
            {item.kind === "plugin" ? <PluginChecks itemId={item.id} /> : null}
            {managedCaptureHook ? (
              <a
                href={routeHref("/onboarding")}
                className="button button--secondary button--small"
              >
                Manage capture
              </a>
            ) : null}
            <section className="manage-section">
              <h3>Where this came from</h3>
              <ProvenanceDetails provenance={item.provenance} label="Source" />
            </section>
          </div>
        )}
      </div>

      {document === null ||
      (!document.editable && !document.removable) ? null : (
        <footer className="tool-panel__footer">
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
          {document.editable ? (
            <>
              {dirty ? (
                <Button
                  size="small"
                  disabled={busy}
                  onClick={() => setDraft(null)}
                >
                  Revert
                </Button>
              ) : null}
              <Button
                size="small"
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
                {busy && mode === "edit" ? "Saving…" : "Save changes"}
              </Button>
            </>
          ) : null}
        </footer>
      )}

      {mode === "discard" ? (
        <Modal
          title="Discard unsaved changes?"
          onClose={() => setMode("edit")}
          footer={
            <>
              <Button onClick={() => setMode("edit")}>Keep editing</Button>
              <Button
                variant="danger"
                onClick={() => {
                  setDraft(null);
                  setMode("edit");
                  leave?.();
                }}
              >
                Discard changes
              </Button>
            </>
          }
        >
          <p>Your edits to {item.name} have not been saved.</p>
        </Modal>
      ) : null}

      {mode === "remove" && document !== null ? (
        <Modal
          title={`Remove ${item.name}?`}
          onClose={() => changeMode("edit")}
          closeDisabled={busy}
          footer={
            <>
              <Button disabled={busy} onClick={() => changeMode("edit")}>
                Cancel
              </Button>
              <Button
                variant="danger"
                disabled={busy || !document.removable}
                onClick={() => {
                  void mutate(
                    () =>
                      commands.removeInventoryItem(item.id, document.revision),
                    false,
                  );
                }}
              >
                <Trash2 aria-hidden="true" />
                {busy ? "Removing…" : "Remove installation"}
              </Button>
            </>
          }
        >
          {error === null ? null : (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <DefinitionList
            rows={[
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
          <Disclosure>
            A recovery copy is kept.
            {item.kind === "skill" ? " Supporting files are removed too." : ""}
          </Disclosure>
        </Modal>
      ) : null}

      {mode === "install" && document !== null && target !== "" ? (
        <Modal
          title={`Install ${item.name} to ${HARNESS_NAMES[target]}`}
          size="wide"
          onClose={() => changeMode("edit")}
          closeDisabled={busy}
          footer={
            <>
              <Button disabled={busy} onClick={() => changeMode("edit")}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={busy || !destination?.available || installBlocked}
                onClick={() => {
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
                {busy ? "Installing…" : `Install to ${HARNESS_NAMES[target]}`}
              </Button>
            </>
          }
        >
          <div className="inventory-manager">
            {error === null ? null : (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
            <Disclosure>
              Creates an independent copy.
              {item.kind === "skill"
                ? " Supporting files are copied with the skill."
                : ""}
            </Disclosure>
            {destination === undefined ? null : (
              <div className="installation-target">
                <strong>
                  <HarnessMark harness={destination.harness} />{" "}
                  {HARNESS_NAMES[destination.harness]} will receive a copy at
                </strong>
                <code>{destination.destination}</code>
              </div>
            )}
            {(destination?.conversions ?? []).length === 0 ? null : (
              <section
                className="manage-section"
                aria-label="Conversions applied"
              >
                <h3>Converted for {HARNESS_NAMES[target]}</h3>
                <ul className="note-list">
                  {destination?.conversions?.map((entry) => (
                    <li key={entry}>{entry}</li>
                  ))}
                </ul>
              </section>
            )}
            {dropped.length === 0 ? null : (
              <section
                className="drop-warning"
                role="group"
                aria-label="Fields that will not be copied"
              >
                <AlertTriangle aria-hidden="true" />
                <div>
                  <strong>These fields will not carry over</strong>
                  <p>
                    {HARNESS_NAMES[target]} has no equivalent for the fields
                    below. The copy will work without them.
                  </p>
                  <ul className="note-list">
                    {dropped.map((field) => (
                      <li key={field}>
                        <code>{field}</code>
                      </li>
                    ))}
                  </ul>
                  <label className="check-row">
                    <input
                      type="checkbox"
                      checked={acknowledgedDrops}
                      disabled={busy}
                      onChange={(event) =>
                        setAcknowledgedDrops(event.currentTarget.checked)
                      }
                    />
                    <span>Install without these fields</span>
                  </label>
                </div>
              </section>
            )}
          </div>
        </Modal>
      ) : null}
    </aside>
  );
}

/** Protocol, approvals and enforced bounds for a Cutokyo plugin. */
function PluginChecks({ itemId }: { readonly itemId: string }) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getPluginVerification(itemId),
    itemId,
  );
  const verification = resource.data;
  if (resource.state === "error") {
    return (
      <ErrorState
        title="Verification could not be read"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  if (verification === null) return <LoadingState label="Loading checks" />;
  return (
    <section className="manage-section" aria-label="Plugin verification">
      <h3>Verification</h3>
      <DefinitionList
        rows={[
          { term: "Protocol", value: `Major ${verification.protocolMajor}` },
          {
            term: "Compatibility",
            value: (
              <StatusPill
                state={
                  verification.protocolState === "compatible"
                    ? "healthy"
                    : "unknown"
                }
              >
                {verification.protocolState}
              </StatusPill>
            ),
          },
          {
            term: "Transcript access",
            value: verification.transcriptApproved
              ? "Approved"
              : "Not approved",
          },
          {
            term: "Network access",
            value: verification.networkApproved ? "Approved" : "Not approved",
          },
          ...verification.limits.map((limit) => ({
            term: limit.label,
            value: limit.value,
          })),
        ]}
      />
      {verification.capabilities.length === 0 ? null : (
        <ul className="check-list" aria-label="Declared capabilities">
          {verification.capabilities.map((value) => (
            <li key={value}>{value}</li>
          ))}
        </ul>
      )}
      <ul className="check-list" aria-label="Checks">
        {verification.evidence.map((value) => (
          <li key={value}>{value}</li>
        ))}
      </ul>
      <Disclosure>{verification.sandboxDisclosure}</Disclosure>
    </section>
  );
}

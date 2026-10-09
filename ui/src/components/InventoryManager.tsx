import {
  AlertTriangle,
  ChevronDown,
  Copy,
  Link2,
  FileText,
  Save,
  Trash2,
} from "lucide-react";
import { useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type { ActionReceipt, Harness, InventoryItem } from "../contracts.js";
import { routeHref } from "../router.js";
import { HARNESS_NAMES, HarnessLabel, HarnessMark } from "./HarnessMark.js";
import { KIND_LABEL, KindIcon } from "./KindIcon.js";
import {
  Button,
  DefinitionList,
  Disclosure,
  ErrorState,
  LoadingState,
  Modal,
  ProvenanceDetails,
} from "./Primitives.js";

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
  const [draft, setDraft] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("edit");
  const [target, setTarget] = useState<Harness | "">("");
  const [acknowledgedDrops, setAcknowledgedDrops] = useState(false);
  const [tab, setTab] = useState<"overview" | "file" | "details">("overview");
  const [menuOpen, setMenuOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
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
  const close = () => {
    if (inFlight.current) return;
    if (mode === "discard") setMode("edit");
    else if (dirty) setMode("discard");
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
  const dropped = destination?.droppedFields ?? [];
  const installBlocked = dropped.length > 0 && !acknowledgedDrops;
  const changeMode = (next: Mode) => {
    setAcknowledgedDrops(false);
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
              ? `Install ${item.name} to ${target === "" ? "another harness" : HARNESS_NAMES[target]}`
              : `Manage ${item.name}`
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
              disabled={
                busy ||
                document === null ||
                !destination?.available ||
                installBlocked
              }
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
                : `Confirm install to ${target === "" ? "harness" : HARNESS_NAMES[target]}`}
            </Button>
          </>
        ) : (
          <>
            <Button disabled={busy} onClick={close}>
              {dirty ? "Cancel" : "Done"}
            </Button>
            {document?.editable && tab === "file" && dirty ? (
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
        <LoadingState label="Loading" />
      ) : resource.state === "error" ? (
        <ErrorState
          title="Installation could not be read"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : document === null || mode === "discard" ? null : (
        <div className="inventory-manager">
          {mode === "remove" ? (
            <>
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
                {item.kind === "skill"
                  ? " Supporting files are removed too."
                  : ""}
              </Disclosure>
            </>
          ) : mode === "install" ? (
            <>
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
                  <h3>
                    Converted for{" "}
                    {destination === undefined
                      ? "this harness"
                      : HARNESS_NAMES[destination.harness]}
                  </h3>
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
                      {destination === undefined
                        ? "This harness"
                        : HARNESS_NAMES[destination.harness]}{" "}
                      has no equivalent for the fields below. The copy will work
                      without them.
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
            </>
          ) : (
            <>
              <header className="manage-hero">
                <div
                  className={`inventory-row__icon inventory-row__icon--${item.kind}`}
                  role="img"
                  aria-label={KIND_LABEL[item.kind]}
                >
                  <KindIcon kind={item.kind} />
                </div>
                <div className="manage-hero__text">
                  <div className="inventory-row__title">
                    <strong>{item.name}</strong>
                    {item.state === "enabled" ||
                    item.state === "installed" ||
                    item.state === "configured" ? null : (
                      <span
                        className={`quiet-badge quiet-badge--${item.state}`}
                      >
                        {item.state}
                      </span>
                    )}
                  </div>
                  {item.description === "" ? null : (
                    <p className="inventory-row__description">
                      {item.description}
                    </p>
                  )}
                </div>
                <div className="install-menu">
                  <Button
                    variant="primary"
                    aria-haspopup="true"
                    aria-expanded={menuOpen}
                    disabled={busy || dirty}
                    onClick={() => setMenuOpen((open) => !open)}
                  >
                    <Copy aria-hidden="true" /> Install to…
                    <ChevronDown aria-hidden="true" />
                  </Button>
                </div>
              </header>
              {menuOpen ? (
                <ul
                  className="install-targets install-menu__list"
                  aria-label="Install to a harness"
                  onKeyDown={(event) => {
                    if (event.key !== "Escape") return;
                    event.preventDefault();
                    event.nativeEvent.stopImmediatePropagation();
                    setMenuOpen(false);
                  }}
                >
                  {document.installTargets.map((entry) => {
                    const installed =
                      item.harnesses.includes(entry.harness) ||
                      /^already installed/i.test(entry.reason ?? "");
                    return (
                      <li key={entry.harness}>
                        <span className="install-targets__name">
                          <HarnessLabel harness={entry.harness} size={16} />
                        </span>
                        <span className="install-targets__state">
                          {installed ? (
                            <span className="quiet-badge quiet-badge--installed">
                              Installed
                            </span>
                          ) : entry.available ? (
                            <>
                              Available
                              {(entry.droppedFields?.length ?? 0) > 0 ? (
                                <span className="install-targets__flag">
                                  {entry.droppedFields?.length} field
                                  {entry.droppedFields?.length === 1
                                    ? ""
                                    : "s"}{" "}
                                  will not carry over
                                </span>
                              ) : null}
                              {(entry.conversions?.length ?? 0) > 0 ? (
                                <span className="install-targets__flag">
                                  {entry.conversions?.length} conversion
                                  {entry.conversions?.length === 1 ? "" : "s"}
                                </span>
                              ) : null}
                            </>
                          ) : (
                            (entry.reason ?? "This format is not supported.")
                          )}
                        </span>
                        {installed ? null : (
                          <Button
                            size="small"
                            variant={entry.available ? "secondary" : "quiet"}
                            disabled={busy || !entry.available}
                            onClick={() => {
                              setMenuOpen(false);
                              setTarget(entry.harness);
                              changeMode("install");
                            }}
                          >
                            Install to {HARNESS_NAMES[entry.harness]}
                          </Button>
                        )}
                      </li>
                    );
                  })}
                </ul>
              ) : null}
              {dirty ? (
                <p className="quiet-label" role="status">
                  Save your changes first.
                </p>
              ) : null}
              <div
                className="pill-tabs"
                role="tablist"
                aria-label="Manage sections"
              >
                {(["overview", "file", "details"] as const).map((id) => (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    id={`manage-tab-${id}`}
                    aria-selected={tab === id}
                    aria-controls="manage-panel"
                    tabIndex={tab === id ? 0 : -1}
                    className={tab === id ? "pill-tab is-active" : "pill-tab"}
                    onClick={() => setTab(id)}
                    onKeyDown={(event) => {
                      const order = ["overview", "file", "details"] as const;
                      const index = order.indexOf(tab);
                      const next =
                        event.key === "ArrowRight"
                          ? order[(index + 1) % order.length]
                          : event.key === "ArrowLeft"
                            ? order[(index + order.length - 1) % order.length]
                            : null;
                      if (next === null || next === undefined) return;
                      event.preventDefault();
                      setTab(next);
                      globalThis.document
                        .getElementById(`manage-tab-${next}`)
                        ?.focus();
                    }}
                  >
                    {id === "overview"
                      ? "Overview"
                      : id === "file"
                        ? "File"
                        : "Details"}
                  </button>
                ))}
              </div>
              <div
                id="manage-panel"
                role="tabpanel"
                aria-labelledby={`manage-tab-${tab}`}
                className="manage-panel"
              >
                {tab === "overview" ? (
                  <>
                    <div
                      className="harness-chips harness-chips--start"
                      role="group"
                      aria-label="Configured harnesses"
                    >
                      {item.harnesses.map((entry) => (
                        <HarnessLabel key={entry} harness={entry} />
                      ))}
                    </div>
                    {(document.notes ?? []).length === 0 ? null : (
                      <ul className="note-list">
                        {document.notes?.map((note) => (
                          <li key={note}>{note}</li>
                        ))}
                      </ul>
                    )}
                    {document.unavailableReason === null ? null : (
                      <Disclosure>{document.unavailableReason}</Disclosure>
                    )}
                    {managedCaptureHook ? (
                      <a
                        href={routeHref("/onboarding")}
                        className="button button--secondary button--small"
                        aria-disabled={busy || dirty}
                        tabIndex={busy || dirty ? -1 : undefined}
                        onClick={(event) => {
                          if (inFlight.current || dirty) {
                            event.preventDefault();
                            return;
                          }
                          onClose();
                        }}
                      >
                        Manage capture
                      </a>
                    ) : null}
                  </>
                ) : tab === "file" ? (
                  <>
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
                        rows={10}
                        onChange={(event) =>
                          setDraft(event.currentTarget.value)
                        }
                      />
                    </label>
                    {document.removable ? (
                      <div className="inventory-manager__actions">
                        <Button
                          size="small"
                          variant="danger"
                          disabled={busy || dirty}
                          onClick={() => changeMode("remove")}
                        >
                          <Trash2 aria-hidden="true" /> Remove
                        </Button>
                      </div>
                    ) : null}
                  </>
                ) : (
                  <>
                    <DefinitionList
                      rows={[
                        { term: "Kind", value: KIND_LABEL[item.kind] },
                        { term: "Scope", value: item.scope },
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
                    <section className="manage-section">
                      <h3>Where this came from</h3>
                      <ProvenanceDetails
                        provenance={item.provenance}
                        label="Source"
                      />
                    </section>
                  </>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </Modal>
  );
}

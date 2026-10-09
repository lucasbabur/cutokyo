import {
  Copy,
  ArrowLeft,
  CalendarDays,
  ExternalLink,
  Filter,
  GitBranch,
  Search,
  Trash2,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useCommands, useCommandResource } from "../commands/context.js";
import type {
  DeletionPreview,
  ResumePreview,
  SessionFilters,
  SessionRecord,
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
  ProvenanceDetails,
  SuccessMessage,
  formatDateTime,
} from "../components/Primitives.js";
import {
  HARNESS_NAMES,
  HarnessFilter,
  HarnessLabel,
  HarnessMark,
} from "../components/HarnessMark.js";
import { RouteNotice } from "../components/Notices.js";
import { formatInteger } from "../domain/reconcile.js";
import { DEFAULT_SESSION_FILTERS } from "../domain/sessionSearch.js";
import { navigate, routeHref } from "../router.js";

const DEFAULT_FILTERS = DEFAULT_SESSION_FILTERS;
const searchStorageKey = () =>
  `cutokyo.session-search:${globalThis.location.search}`;

function retainedFilters(): SessionFilters {
  try {
    const saved: unknown = JSON.parse(
      sessionStorage.getItem(searchStorageKey()) ?? "null",
    );
    if (saved === null || typeof saved !== "object") return DEFAULT_FILTERS;
    const fields = saved as Record<string, unknown>;
    const values = { ...DEFAULT_FILTERS };
    for (const name of [
      "text",
      "project",
      "branch",
      "tool",
      "skill",
      "agent",
    ] as const) {
      if (typeof fields[name] === "string") values[name] = fields[name];
    }
    if (
      ["all", "claude_code", "codex", "opencode"].includes(
        String(fields.harness),
      )
    )
      values.harness = fields.harness as SessionFilters["harness"];
    if (["all", "today", "7d", "30d", "90d"].includes(String(fields.dateRange)))
      values.dateRange = fields.dateRange as SessionFilters["dateRange"];
    if (fields.queryMode === "terms" || fields.queryMode === "phrase")
      values.queryMode = fields.queryMode;
    if (fields.sort === "relevance" || fields.sort === "newest")
      values.sort = fields.sort;
    if (
      Number.isInteger(fields.offset) &&
      Number(fields.offset) >= 0 &&
      Number(fields.offset) <= 4_294_967_295
    )
      values.offset = Number(fields.offset);
    if (
      Number.isInteger(fields.limit) &&
      Number(fields.limit) > 0 &&
      Number(fields.limit) <= 500
    )
      values.limit = Number(fields.limit);
    return values;
  } catch {
    return DEFAULT_FILTERS;
  }
}

export function SessionsPage({
  sessionId,
}: {
  readonly sessionId: string | null;
}) {
  return sessionId === null ? (
    <SessionHistory />
  ) : (
    <SessionDetail sessionId={sessionId} />
  );
}

function SessionHistory() {
  const commands = useCommands();
  const searchRef = useRef<HTMLInputElement>(null);
  const [filters, setFilters] = useState<SessionFilters>(retainedFilters);
  const [debouncedText, setDebouncedText] = useState(filters.text);
  const requested = { ...filters, text: debouncedText };
  const key = JSON.stringify(requested);
  const resource = useCommandResource(
    async () => ({ result: await commands.searchSessions(requested), key }),
    key,
  );
  // Sessions created since the page opened arrive through background imports.
  useEffect(
    () => commands.onHistoryImported(resource.reload),
    [commands, resource.reload],
  );
  useEffect(() => {
    const timer = globalThis.setTimeout(
      () => setDebouncedText(filters.text),
      250,
    );
    return () => globalThis.clearTimeout(timer);
  }, [filters.text]);
  useEffect(() => {
    try {
      sessionStorage.setItem(searchStorageKey(), JSON.stringify(filters));
    } catch {
      /* Storage can be disabled by the host. */
    }
  }, [filters]);
  const stale =
    resource.data !== null && resource.data.key !== JSON.stringify(filters);
  const pending =
    filters.text !== debouncedText || resource.state === "loading";

  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if (
        event.key.toLowerCase() === "k" &&
        (event.metaKey || event.ctrlKey) &&
        !event.altKey
      ) {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    globalThis.addEventListener("keydown", focusSearch);
    return () => globalThis.removeEventListener("keydown", focusSearch);
  }, []);

  const patch = <Key extends keyof SessionFilters>(
    keyName: Key,
    value: SessionFilters[Key],
  ) => {
    setFilters((current) => ({ ...current, offset: 0, [keyName]: value }));
  };
  const filtersActive = Object.entries(filters).some(
    ([keyName, value]) =>
      value !== DEFAULT_FILTERS[keyName as keyof SessionFilters],
  );

  return (
    <div className="page">
      <PageHeader
        title="Sessions"
        actions={
          filtersActive ? (
            <Button
              size="small"
              variant="quiet"
              onClick={() => setFilters(DEFAULT_FILTERS)}
            >
              Clear filters
            </Button>
          ) : undefined
        }
      />

      <section
        className="toolbar toolbar--wrap"
        aria-label="Session search filters"
      >
        <label className="search-field">
          <span className="sr-only">Search session content</span>
          <Search aria-hidden="true" />
          <input
            ref={searchRef}
            type="search"
            aria-label="Search session content"
            aria-keyshortcuts="Control+K Meta+K"
            value={filters.text}
            onChange={(event) => patch("text", event.currentTarget.value)}
            placeholder="Search transcript, session, project…"
          />
          <kbd>Ctrl/⌘ K</kbd>
        </label>
        <label className="inline-field">
          <span>Match</span>
          <select
            aria-label="Match"
            value={filters.queryMode}
            onChange={(event) =>
              patch(
                "queryMode",
                event.currentTarget.value as SessionFilters["queryMode"],
              )
            }
          >
            <option value="terms">All words</option>
            <option value="phrase">Exact phrase</option>
          </select>
        </label>
        <label className="inline-field">
          <span>Sort</span>
          <select
            aria-label="Sort"
            value={filters.sort}
            onChange={(event) =>
              patch("sort", event.currentTarget.value as SessionFilters["sort"])
            }
          >
            <option value="relevance">Most relevant</option>
            <option value="newest">Newest first</option>
          </select>
        </label>
        <HarnessFilter
          value={filters.harness}
          onChange={(value) => patch("harness", value)}
        />
        <label className="inline-field">
          <span>Date</span>
          <select
            aria-label="Date"
            value={filters.dateRange}
            onChange={(event) =>
              patch(
                "dateRange",
                event.currentTarget.value as SessionFilters["dateRange"],
              )
            }
          >
            <option value="all">Any time</option>
            <option value="today">Today</option>
            <option value="7d">Last 7 days</option>
            <option value="30d">Last 30 days</option>
            <option value="90d">Last 90 days</option>
          </select>
        </label>
        <MoreFilters
          data={resource.data?.result ?? null}
          filters={filters}
          patch={patch}
        />
      </section>

      {resource.state === "error" ? (
        <ErrorState
          title="Session search is unavailable"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : null}
      {stale ? (
        <p className="search-stale" role="status">
          Showing previous results.{" "}
          {pending
            ? "Search is updating…"
            : "These results are not for the current search."}
        </p>
      ) : null}
      {resource.state === "loading" && resource.data === null ? (
        <LoadingState label="Searching local session index" />
      ) : resource.data === null ? null : (
        <SessionResults
          data={resource.data.result}
          filters={filters}
          filtersActive={filtersActive}
          patch={patch}
          refreshing={pending || stale}
          clear={() => setFilters(DEFAULT_FILTERS)}
        />
      )}
    </div>
  );
}

function SessionResults({
  data,
  filtersActive,
  patch,
  refreshing,
  clear,
}: {
  readonly clear: () => void;
  readonly data: Awaited<
    ReturnType<ReturnType<typeof useCommands>["searchSessions"]>
  >;
  readonly filters: SessionFilters;
  readonly filtersActive: boolean;
  readonly patch: <Key extends keyof SessionFilters>(
    key: Key,
    value: SessionFilters[Key],
  ) => void;
  readonly refreshing: boolean;
}) {
  return (
    <div className={refreshing ? "is-refreshing" : ""}>
      <RouteNotice meta={data.meta} />

      <div className="results-heading" role="status" aria-live="polite">
        <strong>
          {data.total} {data.total === 1 ? "session" : "sessions"}
        </strong>
        {refreshing ? <span>Updating results…</span> : null}
      </div>

      {data.sessions.length === 0 ? (
        !filtersActive && data.total === 0 ? (
          <EmptyState
            title="No sessions yet"
            description="Sessions from Claude Code, Codex, and OpenCode appear here."
            action={
              <a
                className="button button--primary button--regular"
                href={routeHref("/onboarding")}
              >
                Review capture setup
              </a>
            }
          />
        ) : (
          <EmptyState
            title="No sessions match these filters"
            description={
              data.total > 0
                ? "This page no longer has results. Return to the first page."
                : "Try fewer words, All words instead of Exact phrase, or a broader date range. Your search has been kept."
            }
            action={
              <Button
                onClick={data.total > 0 ? () => patch("offset", 0) : clear}
              >
                {data.total > 0 ? "First page" : "Clear search and filters"}
              </Button>
            }
            compact
          />
        )
      ) : (
        <div className="session-list">
          {data.sessions.map((session) => (
            <SessionRow session={session} key={session.id} />
          ))}
        </div>
      )}
      {data.total > 0 ? (
        <nav className="search-pagination" aria-label="Session search pages">
          <span>
            {data.sessions.length === 0
              ? "No rows on this page"
              : `${data.offset + 1}–${data.offset + data.sessions.length} of ${data.total}`}
          </span>
          <Button
            size="small"
            disabled={refreshing || data.offset === 0}
            onClick={() =>
              patch("offset", Math.max(0, data.offset - data.limit))
            }
          >
            Previous page
          </Button>
          <Button
            size="small"
            disabled={refreshing || !data.hasMore}
            onClick={() => patch("offset", data.offset + data.limit)}
          >
            Next page
          </Button>
        </nav>
      ) : null}
    </div>
  );
}

function FilterSelect({
  label,
  value,
  options,
  onChange,
}: {
  readonly label: string;
  readonly value: string;
  readonly options: readonly string[];
  readonly onChange: (value: string) => void;
}) {
  return (
    <label>
      <span>{label}</span>
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
      >
        <option value="">Any {label.toLocaleLowerCase()}</option>
        {value !== "" && !options.includes(value) ? (
          <option value={value}>{value} (no longer observed)</option>
        ) : null}
        {options.map((option) => (
          <option value={option} key={option}>
            {option}
          </option>
        ))}
      </select>
    </label>
  );
}

function MatchText({
  text,
  terms,
}: {
  readonly text: string;
  readonly terms: readonly string[];
}) {
  const normalize = (word: string) =>
    word.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
  const words = new Set(terms.map(normalize));
  return (
    <>
      {text
        .split(/([\p{L}\p{N}\p{M}]+)/u)
        .map((part, index) =>
          words.has(normalize(part)) ? <mark key={index}>{part}</mark> : part,
        )}
    </>
  );
}

function SessionRow({ session }: { readonly session: SessionRecord }) {
  const usage = session.usage;
  const input =
    usage.length > 0 && usage.every((record) => record.inputTokens !== null)
      ? usage.reduce((total, record) => total + (record.inputTokens ?? 0), 0)
      : null;
  const output =
    usage.length > 0 && usage.every((record) => record.outputTokens !== null)
      ? usage.reduce((total, record) => total + (record.outputTokens ?? 0), 0)
      : null;
  return (
    <article className="session-row">
      <div
        className="session-row__harness"
        data-harness={session.harness}
        title={HARNESS_NAMES[session.harness]}
      >
        <HarnessMark harness={session.harness} />
        <span className="sr-only">{HARNESS_NAMES[session.harness]}</span>
      </div>
      <div className="session-row__body">
        <div className="session-row__title">
          <a href={routeHref("/sessions", session.id)}>
            {session.title ?? "Untitled session"}
          </a>
        </div>
        {session.matches.length > 0 ? (
          <div className="session-matches">
            {session.matches.map((match) => (
              <p key={match.source}>
                <span className="session-match-source">
                  {match.source === "native_id" ? "Session ID" : match.source}
                </span>{" "}
                <MatchText text={match.text} terms={match.terms} />
              </p>
            ))}
          </div>
        ) : session.summary === null ? null : (
          <p>{session.summary}</p>
        )}
        <div className="session-row__meta">
          <span title={session.project ?? undefined}>
            <span>{session.project ?? "Project unknown"}</span>
          </span>
          <span className="session-row__date">
            <CalendarDays aria-hidden="true" />
            <span>{formatDateTime(session.startedAt)}</span>
          </span>
          <span title={session.branch ?? undefined}>
            <GitBranch aria-hidden="true" />
            <span>{session.branch ?? "Branch unknown"}</span>
          </span>
        </div>
      </div>
      <div
        className="session-row__usage"
        role="group"
        aria-label="Session usage"
      >
        <span>
          <strong>{formatInteger(input)}</strong> in
        </span>
        <span>
          <strong>{formatInteger(output)}</strong> out
        </span>
      </div>
      <a
        className="icon-link"
        href={routeHref("/sessions", session.id)}
        aria-label={`Open ${session.title ?? session.id}`}
      >
        <ExternalLink aria-hidden="true" />
      </a>
    </article>
  );
}

function SessionDetail({ sessionId }: { readonly sessionId: string }) {
  const commands = useCommands();
  const resource = useCommandResource(
    () => commands.getSession(sessionId),
    sessionId,
  );
  const announce = useAnnounce();
  const [resume, setResume] = useState<ResumePreview | "loading" | null>(null);
  const [deletion, setDeletion] = useState<DeletionPreview | "loading" | null>(
    null,
  );
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<string | null>(null);
  const resumeInFlight = useRef(false);

  const closeResume = () => {
    if (resumeInFlight.current) return;
    setResume(null);
    setActionError(null);
  };
  const openResume = async () => {
    if (resumeInFlight.current) return;
    setReceipt(null);
    setActionError(null);
    setResume("loading");
    try {
      setResume(await commands.previewResume(sessionId));
    } catch (reason) {
      setResume(null);
      setActionError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const executeResume = async () => {
    if (
      resumeInFlight.current ||
      resume === null ||
      resume === "loading" ||
      !resume.canResume
    )
      return;
    resumeInFlight.current = true;
    setActionBusy(true);
    setActionError(null);
    setReceipt(null);
    try {
      const result = await commands.resumeSession(sessionId);
      if (!result.ok) throw new Error(result.message);
      setReceipt(result.message);
      announce(result.message);
      setResume(null);
    } catch (reason) {
      setActionError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      resumeInFlight.current = false;
      setActionBusy(false);
    }
  };
  const openDeletion = async () => {
    setActionError(null);
    setDeletion("loading");
    try {
      setDeletion(await commands.previewSessionDeletion(sessionId));
    } catch (reason) {
      setDeletion(null);
      setActionError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const executeDeletion = async () => {
    if (deletion === null || deletion === "loading") return;
    setActionBusy(true);
    try {
      const result = await commands.deleteSession(
        sessionId,
        deletion.previewToken,
      );
      announce(
        `Deleted ${result.sessions} session and ${result.ftsRows} search rows.`,
      );
      setDeletion(null);
      navigate("/sessions");
    } catch (reason) {
      setActionError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setActionBusy(false);
    }
  };

  if (resource.state === "loading" && resource.data === null) {
    return <LoadingState label="Loading session" />;
  }
  if (resource.state === "error" && resource.data === null) {
    return (
      <ErrorState
        title="Session detail is unavailable"
        error={resource.error}
        onRetry={resource.reload}
      />
    );
  }
  const session = resource.data;
  if (session === null) return null;

  return (
    <div className="page">
      <a className="back-link" href={routeHref("/sessions")}>
        <ArrowLeft aria-hidden="true" /> Back to sessions
      </a>
      <PageHeader
        title={session.title ?? "Untitled session"}
        actions={
          <>
            <Button onClick={() => void openResume()}>
              <ExternalLink aria-hidden="true" /> Resume
            </Button>
            <Button variant="danger" onClick={() => void openDeletion()}>
              <Trash2 aria-hidden="true" /> Delete session
            </Button>
          </>
        }
      />
      {actionError === null || resume !== null ? null : (
        <p className="form-error" role="alert">
          {actionError}
        </p>
      )}
      {receipt === null ? null : <SuccessMessage>{receipt}</SuccessMessage>}

      <div className="detail-grid">
        <section
          className="panel panel--span-2"
          aria-labelledby="timeline-heading"
        >
          <div className="panel__header">
            <div>
              <h2 id="timeline-heading">Timeline</h2>
            </div>
          </div>
          {session.timeline.length === 0 ? (
            <EmptyState
              compact
              title="No messages"
              description="This session has no readable messages."
            />
          ) : (
            <ol className="timeline">
              {session.timeline.map((entry) => (
                <li key={entry.id}>
                  <div
                    className={`timeline__mark timeline__mark--${entry.kind}`}
                    aria-hidden="true"
                  />
                  <article>
                    <div className="timeline__heading">
                      <span className="timeline__kind">{entry.kind}</span>
                      <time dateTime={entry.at}>
                        {formatDateTime(entry.at)}
                      </time>
                    </div>
                    <h3>{entry.title}</h3>
                    {entry.body === null ? null : <p>{entry.body}</p>}
                  </article>
                </li>
              ))}
            </ol>
          )}
        </section>

        <aside
          className="panel detail-facts"
          aria-labelledby="identity-heading"
        >
          <div className="panel__header">
            <h2 id="identity-heading">Session</h2>
          </div>
          <DefinitionList
            rows={[
              {
                term: "Harness",
                value: <HarnessLabel harness={session.harness} />,
              },
              { term: "Project", value: session.project ?? "—" },
              { term: "Branch", value: session.branch ?? "—" },
              {
                term: "Session ID",
                value: (
                  <span className="copy-row">
                    <code
                      title={session.nativeResumeId ?? session.id}
                      aria-label={session.nativeResumeId ?? session.id}
                    >
                      {session.nativeResumeId ?? session.id}
                    </code>
                    <Button
                      size="small"
                      variant="quiet"
                      aria-label="Copy session ID"
                      onClick={() => {
                        void globalThis.navigator.clipboard
                          ?.writeText(session.nativeResumeId ?? session.id)
                          .then(() => announce("Session ID copied"));
                      }}
                    >
                      <Copy aria-hidden="true" />
                    </Button>
                  </span>
                ),
              },
            ]}
          />
          <details className="advanced">
            <summary>Details</summary>
            <DefinitionList
              rows={[
                { term: "State", value: session.state },
                {
                  term: "Coverage",
                  value: `${session.provenance.coverage.state.replaceAll("_", " ")} · ${session.provenance.coverage.scope}`,
                },
                {
                  term: "Session tree",
                  value: <code>{session.nativeSessionKey}</code>,
                },
                {
                  term: "Resume target",
                  value:
                    session.nativeResumeId === null ? (
                      "—"
                    ) : (
                      <code>{session.nativeResumeId}</code>
                    ),
                },
              ]}
            />
            <ProvenanceDetails provenance={session.provenance} label="Source" />
          </details>
        </aside>
      </div>

      {resume === null ? null : (
        <Modal
          title={`Resume in ${resume === "loading" ? "…" : resume.harnessName}?`}
          onClose={closeResume}
          closeDisabled={actionBusy}
          footer={
            <>
              <Button disabled={actionBusy} onClick={closeResume}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={
                  resume === "loading" || !resume.canResume || actionBusy
                }
                onClick={() => void executeResume()}
              >
                {actionBusy ? "Opening…" : "Resume"}
              </Button>
            </>
          }
        >
          {actionError === null ? null : (
            <p className="form-error" role="alert">
              {actionError}
            </p>
          )}
          {resume === "loading" ? (
            <LoadingState label="Checking" />
          ) : (
            <>
              <p className="dialog-line">
                Folder: {resume.projectDirectory ?? "No folder recorded"}
              </p>
              {resume.canResume ? null : <p>{resume.unavailableReason}</p>}
            </>
          )}
        </Modal>
      )}

      {deletion === null ? null : (
        <Modal
          title="Delete only this session?"
          onClose={() => setDeletion(null)}
          footer={
            <>
              <Button onClick={() => setDeletion(null)}>Cancel</Button>
              <Button
                variant="danger"
                disabled={deletion === "loading" || actionBusy}
                onClick={() => void executeDeletion()}
              >
                {actionBusy ? "Deleting…" : "Delete selected session"}
              </Button>
            </>
          }
        >
          {deletion === "loading" ? (
            <LoadingState label="Checking" />
          ) : (
            <>
              <DefinitionList
                rows={[
                  { term: "Session", value: deletion.sessionTitles.join(", ") },
                  { term: "Messages", value: deletion.messages },
                ]}
              />
              <Disclosure>{deletion.disclosure}</Disclosure>
            </>
          )}
        </Modal>
      )}
    </div>
  );
}

function MoreFilters({
  data,
  filters,
  patch,
}: {
  readonly data: Awaited<
    ReturnType<ReturnType<typeof useCommands>["searchSessions"]>
  > | null;
  readonly filters: SessionFilters;
  readonly patch: <Key extends keyof SessionFilters>(
    key: Key,
    value: SessionFilters[Key],
  ) => void;
}) {
  const activeConstraints = (
    [
      ["Project", filters.project],
      ["Branch", filters.branch],
      ["Tool", filters.tool],
      ["Skill", filters.skill],
      ["Agent", filters.agent],
    ] as const
  ).filter(([, value]) => value !== "");
  return (
    <details className="advanced-filters">
      <summary>
        <Filter aria-hidden="true" /> More filters
        {activeConstraints.length > 0 ? (
          <>
            <span className="filter-count">Active</span>
            <span
              className="advanced-filters__summary"
              aria-label="Active additional filters"
            >
              {activeConstraints.map(([label, value]) => (
                <span key={label}>
                  <strong>{label}:</strong> {value}
                </span>
              ))}
            </span>
          </>
        ) : null}
      </summary>
      <div className="advanced-filters__grid">
        <FilterSelect
          label="Project"
          value={filters.project}
          options={data?.availableProjects ?? []}
          onChange={(value) => patch("project", value)}
        />
        <FilterSelect
          label="Branch"
          value={filters.branch}
          options={data?.availableBranches ?? []}
          onChange={(value) => patch("branch", value)}
        />
        <FilterSelect
          label="Tool"
          value={filters.tool}
          options={data?.availableTools ?? []}
          onChange={(value) => patch("tool", value)}
        />
        <FilterSelect
          label="Skill"
          value={filters.skill}
          options={data?.availableSkills ?? []}
          onChange={(value) => patch("skill", value)}
        />
        <FilterSelect
          label="Agent"
          value={filters.agent}
          options={data?.availableAgents ?? []}
          onChange={(value) => patch("agent", value)}
        />
      </div>
    </details>
  );
}

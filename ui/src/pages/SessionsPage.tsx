import {
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
  Harness,
  ResumePreview,
  SessionFilters,
  SessionRecord,
} from "../contracts.js";
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
  ProvenanceDetails,
  RouteNotice,
  StatusPill,
  SuccessMessage,
  formatDateTime,
} from "../components/Primitives.js";
import { formatInteger } from "../domain/reconcile.js";
import { navigate, routeHref } from "../router.js";

const DEFAULT_FILTERS: SessionFilters = {
  text: "",
  harness: "all",
  project: "",
  branch: "",
  dateRange: "all",
  tool: "",
  skill: "",
  agent: "",
};

const HARNESS_NAMES: Record<Harness, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
};

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
  const [filters, setFilters] = useState<SessionFilters>(DEFAULT_FILTERS);
  const key = JSON.stringify(filters);
  const resource = useCommandResource(
    () => commands.searchSessions(filters),
    key,
  );

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
    setFilters((current) => ({ ...current, [keyName]: value }));
  };
  const filtersActive = Object.entries(filters).some(
    ([keyName, value]) =>
      value !== DEFAULT_FILTERS[keyName as keyof SessionFilters],
  );

  return (
    <div className="page">
      <PageHeader
        eyebrow="Native session history"
        title="Sessions"
        description="Search transcript text and filter attributable local history without losing exact native resume identity."
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

      <section className="filter-bar" aria-label="Session search filters">
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
        <label>
          <span>Harness</span>
          <select
            value={filters.harness}
            onChange={(event) =>
              patch("harness", event.currentTarget.value as Harness | "all")
            }
          >
            <option value="all">All harnesses</option>
            <option value="claude_code">Claude Code</option>
            <option value="codex">Codex</option>
            <option value="opencode">OpenCode</option>
          </select>
        </label>
        <label>
          <span>Date</span>
          <select
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
      </section>

      {resource.state === "loading" && resource.data === null ? (
        <LoadingState label="Searching local session index" />
      ) : resource.state === "error" && resource.data === null ? (
        <ErrorState
          title="Session search is unavailable"
          error={resource.error}
          onRetry={resource.reload}
        />
      ) : resource.data === null ? null : (
        <SessionResults
          data={resource.data}
          filters={filters}
          filtersActive={filtersActive}
          patch={patch}
          refreshing={resource.state === "loading"}
        />
      )}
    </div>
  );
}

function SessionResults({
  data,
  filters,
  filtersActive,
  patch,
  refreshing,
}: {
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
      <details className="advanced-filters">
        <summary>
          <Filter aria-hidden="true" /> More filters
          {filtersActive ? <span className="filter-count">Active</span> : null}
        </summary>
        <div className="advanced-filters__grid">
          <FilterSelect
            label="Project"
            value={filters.project}
            options={data.availableProjects}
            onChange={(value) => patch("project", value)}
          />
          <FilterSelect
            label="Branch"
            value={filters.branch}
            options={data.availableBranches}
            onChange={(value) => patch("branch", value)}
          />
          <FilterSelect
            label="Tool"
            value={filters.tool}
            options={data.availableTools}
            onChange={(value) => patch("tool", value)}
          />
          <FilterSelect
            label="Skill"
            value={filters.skill}
            options={data.availableSkills}
            onChange={(value) => patch("skill", value)}
          />
          <FilterSelect
            label="Agent"
            value={filters.agent}
            options={data.availableAgents}
            onChange={(value) => patch("agent", value)}
          />
        </div>
      </details>

      <div className="results-heading" role="status" aria-live="polite">
        <strong>
          {data.total} {data.total === 1 ? "session" : "sessions"}
        </strong>
        <span>{refreshing ? "Updating results…" : "Newest first"}</span>
      </div>

      {data.sessions.length === 0 ? (
        data.availableProjects.length === 0 ? (
          <EmptyState
            title="History is empty—not zero"
            description="No local session evidence has been captured. Check native setup and system health; Cutokyo will not fabricate empty usage totals while coverage is unavailable."
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
            description="The local index was searched successfully. Try a broader date range or clear project, branch, tool, skill, and agent filters."
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
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
      >
        <option value="">Any {label.toLocaleLowerCase()}</option>
        {options.map((option) => (
          <option value={option} key={option}>
            {option}
          </option>
        ))}
      </select>
    </label>
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
      <div className="session-row__harness" data-harness={session.harness}>
        <span aria-hidden="true" />
        {HARNESS_NAMES[session.harness]}
      </div>
      <div className="session-row__body">
        <div className="session-row__title">
          <a href={routeHref("/sessions", session.id)}>
            {session.title ?? "Untitled session"}
          </a>
          <CoverageBadge coverage={session.provenance.coverage} />
        </div>
        <p>{session.summary ?? "Transcript summary unavailable."}</p>
        <div className="session-row__meta">
          <span>
            <CalendarDays aria-hidden="true" />{" "}
            {formatDateTime(session.startedAt)}
          </span>
          <span>
            <GitBranch aria-hidden="true" />{" "}
            {session.branch ?? "Branch unknown"}
          </span>
          <span>{session.project ?? "Project unknown"}</span>
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

  const openResume = async () => {
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
    setActionBusy(true);
    try {
      const result = await commands.resumeSession(sessionId);
      setReceipt(result.message);
      announce(result.message);
      setResume(null);
    } catch (reason) {
      setActionError(reason instanceof Error ? reason.message : String(reason));
    } finally {
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
    return <LoadingState label="Loading attributable session timeline" />;
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
        eyebrow={`${HARNESS_NAMES[session.harness]} · ${session.state}`}
        title={session.title ?? "Untitled session"}
        description={
          session.summary ?? "No local summary is available for this session."
        }
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
      {actionError === null ? null : (
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
              <p className="eyebrow">Chronological evidence</p>
              <h2 id="timeline-heading">Timeline</h2>
            </div>
            <CoverageBadge coverage={session.provenance.coverage} />
          </div>
          {session.timeline.length === 0 ? (
            <EmptyState
              compact
              title="Timeline content unavailable"
              description="The native source established this session identity but did not expose attributable message or tool events."
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
                    <ProvenanceDetails
                      provenance={entry.provenance}
                      label="Event provenance"
                    />
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
            <div>
              <p className="eyebrow">Exact identity</p>
              <h2 id="identity-heading">Native target</h2>
            </div>
          </div>
          <DefinitionList
            rows={[
              { term: "Harness", value: HARNESS_NAMES[session.harness] },
              {
                term: "Session tree",
                value: <code>{session.nativeSessionKey}</code>,
              },
              {
                term: "Resume target",
                value:
                  session.nativeResumeId === null ? (
                    <StatusPill state="unavailable">Unavailable</StatusPill>
                  ) : (
                    <code>{session.nativeResumeId}</code>
                  ),
              },
              { term: "Project", value: session.project ?? "Unknown" },
              { term: "Branch", value: session.branch ?? "Unknown" },
            ]}
          />
          <ProvenanceDetails provenance={session.provenance} />
        </aside>
      </div>

      {resume === null ? null : (
        <Modal
          title="Resume this exact native session?"
          description="Cutokyo will pass only the recorded native resume target to the selected harness."
          onClose={() => setResume(null)}
          footer={
            <>
              <Button onClick={() => setResume(null)}>Cancel</Button>
              <Button
                variant="primary"
                disabled={
                  resume === "loading" || !resume.canResume || actionBusy
                }
                onClick={() => void executeResume()}
              >
                {actionBusy
                  ? "Opening…"
                  : `Resume in ${resume === "loading" ? "harness" : resume.harnessName}`}
              </Button>
            </>
          }
        >
          {resume === "loading" ? (
            <LoadingState label="Resolving exact native target" />
          ) : (
            <>
              <DefinitionList
                rows={[
                  { term: "Harness", value: resume.harnessName },
                  {
                    term: "Native session",
                    value: <code>{resume.nativeResumeId}</code>,
                  },
                  { term: "Action", value: resume.commandDescription },
                ]}
              />
              {resume.canResume ? null : (
                <Disclosure>{resume.unavailableReason}</Disclosure>
              )}
            </>
          )}
        </Modal>
      )}

      {deletion === null ? null : (
        <Modal
          title="Delete only this session?"
          description="Review the exact transactional scope before removing local evidence."
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
            <LoadingState label="Previewing linked rows" />
          ) : (
            <>
              <DefinitionList
                rows={[
                  { term: "Session", value: deletion.sessionTitles.join(", ") },
                  { term: "Raw observations", value: deletion.rawObservations },
                  { term: "Messages", value: deletion.messages },
                  { term: "Summaries", value: deletion.summaries },
                  { term: "Search rows", value: deletion.ftsRows },
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

"use client";

import { Fragment, useEffect, useRef, useState } from "react";

import {
  harnesses,
  harnessNames,
  harnessPaths,
  inventoryTools,
  isPortable,
  kinds,
  kindNames,
  sourceContent,
} from "./inventory-data";
import styles from "./inventory-demo.module.css";

import type { Harness, InventoryTool } from "./inventory-data";

const iconPaths: Record<string, string> = {
  all: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  skill:
    "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3z M20 2v4 M18 4h4 M3 2v4 M1 4h4",
  mcp: "M8 3v6 M16 3v6 M6 9h12v4a6 6 0 0 1-12 0V9z M12 19v3",
  hook: "M12 8a3 3 0 1 0-3-3 M5 15a3 3 0 1 0 3 3 M16 18a3 3 0 1 0 3-3 M10 7 5 15 M8 18h8 M19 15l-5-8",
  plugin: "M8 3h3a3 3 0 1 1 6 0h4v7a3 3 0 1 0 0 6v5h-7a3 3 0 1 0-6 0H3v-7a3 3 0 1 0 0-6V3h5z",
  instruction: "M14 2H5v20h14V7l-5-5z M14 2v6h5 M8 12h8 M8 16h8",
  warning: "m12 3 10 18H2L12 3z M12 9v5 M12 17v.5",
  plus: "M12 5v14 M5 12h14",
  search: "M21 21l-5-5 M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
  refresh: "M20 7v5h-5 M4 17v-5h5 M5 7a8 8 0 0 1 14-2l1 2 M4 17l1 2a8 8 0 0 0 14-2",
  close: "m6 6 12 12 M6 18 18 6",
  overview: "M3 18a10 10 0 1 1 18 0 M12 13l5-5",
  sessions: "M3 3v6h6 M3 9a9 9 0 1 1 0 6 M12 7v5l4 2",
  tools:
    "m12 2 5 3v6l-5 3-5-3V5l5-3z M7 11l-5 3v6l5 3 5-3v-6 M17 11l5 3v6l-5 3-5-3 M2 14l5 3 5-3 5 3 5-3 M7 17v6 M17 17v6 M7 5l5 3 5-3 M12 8v6",
  health:
    "M4 3v6a5 5 0 0 0 10 0V3 M2 3h4 M12 3h4 M9 14v3a5 5 0 0 0 10 0v-4 M21 10a2 2 0 1 1-4 0 2 2 0 0 1 4 0",
  data: "M3 5c0-4 18-4 18 0s-18 4-18 0v14c0 4 18 4 18 0V5 M3 12c0 4 18 4 18 0",
  settings:
    "m10 2-1 3-3 1-3 1 1 4-2 2 2 2-1 4 4 1 2 2 3-1 3 1 2-3 3-1-1-4 1-3-3-2-1-3-4 1-2-1z M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
  bell: "M4 17h16l-2-4V9a6 6 0 0 0-12 0v4l-2 4z M9 21h6 M2 8l2-4 M20 4l2 4",
  play: "m8 4 12 8-12 8V4z",
  pause: "M8 4v16 M16 4v16",
  expand: "M14 3h7v7 M21 3l-7 7 M10 21H3v-7 M3 21l7-7",
};

function Icon({ name }: { name: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={iconPaths[name] ?? iconPaths.all} />
    </svg>
  );
}

function HarnessMark({ harness }: { harness: Harness }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" fillRule="evenodd" aria-hidden="true">
      <path d={harnessPaths[harness]} />
    </svg>
  );
}

type DemoState = {
  query: string;
  filter: string;
  selected: string | null;
  tab: "file" | "details";
};
const initialState: DemoState = { query: "", filter: "all", selected: null, tab: "file" };

// The cursor arrives first; each following action updates the same working controls.
// eslint-disable-next-line local/no-complex-business-logic -- This is a presentation-only animation timeline, with no backend operations.
function sceneAt(time: number): DemoState & { target: string; click: boolean } {
  const query =
    time < 3000 ? "" : time < 13500 ? "Cutokyo".slice(0, Math.floor((time - 3000) / 150)) : "";
  const selected =
    time >= 6200 && time < 12000 ? "search" : time >= 17300 && time < 21800 ? "docs" : null;
  const tab =
    (time >= 8500 && time < 10300) || (time >= 19300 && time < 21000) ? "details" : "file";
  const filter = time >= 15000 && time < 23200 ? "mcp" : "all";
  const targets: [number, string][] = [
    [0, "all"],
    [1700, "search-input"],
    [5100, "row-search"],
    [7700, "details"],
    [9600, "file"],
    [11300, "close"],
    [12500, "search-input"],
    [14200, "mcp"],
    [16400, "row-docs"],
    [18500, "details"],
    [20400, "file"],
    [21100, "close"],
    [22500, "all"],
  ];
  const target =
    targets
      .slice()
      .reverse()
      .find(([start]) => time >= start)?.[1] ?? "all";
  const click = [
    2800, 6200, 8500, 10300, 12000, 13400, 15000, 17300, 19300, 21000, 21800, 23200,
  ].some((at) => time >= at && time < at + 360);
  return { query, selected, tab, filter, target, click };
}

// eslint-disable-next-line local/no-complex-business-logic -- Filters and local preview state reproduce the desktop interface without mutating user data.
export function InventoryDemo() {
  const root = useRef<HTMLDivElement>(null);
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(true);
  const [visible, setVisible] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [manual, setManual] = useState<DemoState>(initialState);
  const [tools, setTools] = useState(inventoryTools);
  const [enabled, setEnabled] = useState(true);
  const [cursor, setCursor] = useState({ x: 260, y: 94 });
  const [expanded, setExpanded] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [saved, setSaved] = useState<Record<string, string>>({});
  const scene = sceneAt(time);
  const automatic = playing && !reducedMotion;
  const state = automatic ? scene : manual;
  const selected = tools.find((tool) => tool.id === state.selected);
  const queryTools = tools.filter((tool) =>
    `${tool.name} ${tool.description} ${tool.origin} ${tool.scope}`
      .toLowerCase()
      .includes(state.query.toLowerCase()),
  );
  const matchesFilter = (tool: InventoryTool, filter: string) =>
    filter === "all" ||
    tool.kind === filter ||
    tool.harnesses.some((harness) => harness === filter) ||
    (filter === "attention" && tool.warning) ||
    (filter === "coverage" && isPortable(tool) && tool.harnesses.length < 3);
  const filtered = queryTools.filter((tool) => matchesFilter(tool, state.filter));

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(media.matches);
    update();
    media.addEventListener("change", update);
    const observer = new IntersectionObserver(
      ([entry]) => setVisible(entry?.isIntersecting ?? false),
      { threshold: 0.2 },
    );
    if (root.current) observer.observe(root.current);
    return () => {
      observer.disconnect();
      media.removeEventListener("change", update);
    };
  }, []);

  useEffect(() => {
    if (!automatic || !visible) return;
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      const elapsed = now - previous;
      previous = now;
      if (!document.hidden) setTime((current) => (current + elapsed) % 27000);
    }, 100);
    return () => window.clearInterval(timer);
  }, [automatic, visible]);

  useEffect(() => {
    if (!automatic || !visible) return;
    const update = () => {
      const element = root.current?.querySelector<HTMLElement>(
        `[data-demo-target="${scene.target}"]`,
      );
      const bounds = root.current?.getBoundingClientRect();
      if (!element || !bounds) return;
      const rail = element.closest<HTMLElement>(`.${styles.filters}`);
      if (rail && rail.scrollWidth > rail.clientWidth) {
        rail.scrollLeft = element.offsetLeft - rail.offsetLeft - 24;
      }
      const target = element.getBoundingClientRect();
      setCursor({
        x: target.left - bounds.left + Math.min(target.width * 0.55, 110),
        y: target.top - bounds.top + target.height * 0.55,
      });
    };
    update();
    const observer = new ResizeObserver(update);
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, [automatic, visible, scene.target, scene.selected]);

  function interact(patch: Partial<DemoState> = {}) {
    setManual({ ...state, ...patch });
    setPlaying(false);
  }

  function select(id: string) {
    interact({ selected: id, tab: "file" });
  }
  function install(tool: InventoryTool, harness: Harness) {
    interact();
    setTools((current) =>
      current.map((entry) =>
        entry.id === tool.id
          ? { ...entry, harnesses: [...new Set([...entry.harnesses, harness])] }
          : entry,
      ),
    );
  }

  function filterButton(id: string, label: string, icon: string, harness?: Harness) {
    return (
      <button
        type="button"
        data-demo-target={id}
        className={id === "attention" ? styles.attention : undefined}
        aria-pressed={state.filter === id}
        onClick={() => interact({ filter: id, selected: null })}
      >
        {harness ? <HarnessMark harness={harness} /> : <Icon name={icon} />}
        <span>{label}</span>
        <small>{queryTools.filter((tool) => matchesFilter(tool, id)).length}</small>
      </button>
    );
  }

  const fileContent = selected
    ? (drafts[selected.id] ?? saved[selected.id] ?? sourceContent(selected))
    : "";

  return (
    <div
      className={styles.demo}
      ref={root}
      aria-label="Cutokyo Agent tools interactive preview"
      data-inventory-demo
    >
      <div className={styles.app}>
        <aside className={styles.sidebar} aria-label="Cutokyo navigation preview">
          <div className={styles.brand}>
            <svg viewBox="0 0 1024 1024" aria-hidden="true">
              <rect width="1024" height="1024" rx="112" fill="#050607" />
              <path
                fill="#f5f4ea"
                d="M224 200h580v204l-128 52V336H348v352h328V568l128 52v204H224V200Z"
              />
              <circle cx="512" cy="512" r="112" fill="#c3f400" />
              <circle cx="512" cy="512" r="64" fill="#050607" />
            </svg>
            <strong>Cutokyo</strong>
          </div>
          <div className={styles.navigation}>
            {[
              ["overview", "Overview"],
              ["sessions", "Sessions"],
              ["tools", "Agent tools"],
              ["health", "Health"],
              ["data", "Data"],
              ["settings", "Settings"],
            ].map(([icon, label]) => (
              <div key={label} className={icon === "tools" ? styles.navActive : undefined}>
                <Icon name={icon ?? "all"} />
                <span>{label}</span>
              </div>
            ))}
          </div>
          <div className={styles.sidebarFooter}>
            <span>
              <Icon name="bell" />
              Warnings<b>1</b>
            </span>
            <small>v0.1.0-test</small>
          </div>
        </aside>
        <div className={styles.workspace}>
          <header className={styles.header}>
            <h3>Agent tools</h3>
            <label
              className={styles.search}
              data-searching={automatic && scene.target === "search-input"}
            >
              <Icon name="search" />
              <input
                data-demo-target="search-input"
                aria-label="Search installed tools"
                placeholder="Search tools"
                value={state.query}
                onFocus={() => interact()}
                onChange={(event) => interact({ query: event.target.value, selected: null })}
              />
            </label>
            <button
              className={styles.iconButton}
              type="button"
              aria-label="Rescan installations"
              onClick={() => {
                interact(initialState);
                setTools(inventoryTools);
                setEnabled(true);
              }}
            >
              <Icon name="refresh" />
            </button>
          </header>
          <div className={`${styles.layout} ${selected ? styles.hasPanel : ""}`}>
            <aside className={styles.filters} aria-label="Filter tools">
              {filterButton("all", "All tools", "all")}
              {filterButton("attention", "Needs attention", "warning")}
              <p>Type</p>
              {kinds.map((kind) => (
                <Fragment key={kind}>{filterButton(kind, kindNames[kind], kind)}</Fragment>
              ))}
              <p>Harness</p>
              {harnesses.map((harness) => (
                <Fragment key={harness}>
                  {filterButton(harness, harnessNames[harness], "", harness)}
                </Fragment>
              ))}
              <p>Coverage</p>
              {filterButton("coverage", "Not everywhere", "plus")}
            </aside>
            <div className={styles.tableWrap}>
              <table className={styles.table} aria-label="Installed agent tools">
                <thead>
                  <tr>
                    <th scope="col">Name</th>
                    {harnesses.map((harness) => (
                      <th scope="col" key={harness}>
                        <span title={harnessNames[harness]}>
                          <HarnessMark harness={harness} />
                          <span className={styles.harnessName}>{harnessNames[harness]}</span>
                        </span>
                      </th>
                    ))}
                  </tr>
                </thead>
                {kinds.map((kind) => {
                  const items = filtered.filter((tool) => tool.kind === kind);
                  if (!items.length) return null;
                  return (
                    <tbody key={kind}>
                      <tr className={styles.group}>
                        <th colSpan={4} scope="colgroup">
                          {kindNames[kind]} <span>{items.length}</span>
                        </th>
                      </tr>
                      {items.map((tool) => (
                        <tr
                          key={tool.id}
                          className={styles.row}
                          data-selected={selected?.id === tool.id}
                        >
                          <td>
                            <button
                              type="button"
                              data-demo-target={`row-${tool.id}`}
                              className={styles.toolName}
                              onClick={() => select(tool.id)}
                              aria-label={`View ${tool.name}`}
                              aria-expanded={selected?.id === tool.id}
                            >
                              <span className={styles.toolIcon}>
                                <Icon name={tool.kind} />
                              </span>
                              <span className={styles.toolCopy}>
                                <span className={styles.toolTitle}>
                                  <strong>{tool.name}</strong>
                                  {tool.scope !== "user" && <small>{tool.scope}</small>}
                                  {tool.managed && <em>Cutokyo</em>}
                                  {tool.warning && <b>degraded</b>}
                                </span>
                                <span className={styles.description}>{tool.description}</span>
                              </span>
                            </button>
                            {tool.id === "docs" && (
                              <button
                                type="button"
                                role="switch"
                                aria-checked={enabled}
                                aria-label="Enable docs-73A9"
                                className={styles.switch}
                                onClick={() => {
                                  interact();
                                  setEnabled(!enabled);
                                }}
                              >
                                <span />
                              </button>
                            )}
                          </td>
                          {harnesses.map((harness) => (
                            <td key={harness}>
                              {tool.harnesses.includes(harness) ? (
                                <span
                                  title={`${harnessNames[harness]}: ${tool.id === "docs" && !enabled ? "disabled" : "installed"}`}
                                  className={styles.dot}
                                  data-harness={harness}
                                  data-off={tool.id === "docs" && !enabled}
                                />
                              ) : isPortable(tool) ? (
                                <button
                                  className={styles.add}
                                  type="button"
                                  aria-label={`Manage ${tool.name} for ${harnessNames[harness]}`}
                                  onClick={() => select(tool.id)}
                                >
                                  <Icon name="plus" />
                                </button>
                              ) : (
                                <span className={styles.dash}>–</span>
                              )}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  );
                })}
              </table>
              {!filtered.length && (
                <div className={styles.empty}>
                  <Icon name="search" />
                  <strong>No tools found</strong>
                  <p>Try another search or filter.</p>
                  <button type="button" onClick={() => interact(initialState)}>
                    Clear filters
                  </button>
                </div>
              )}
            </div>
            {selected && (
              <aside className={styles.panel} aria-label={`${selected.name} details`}>
                <header>
                  <span className={styles.toolIcon}>
                    <Icon name={selected.kind} />
                  </span>
                  <div>
                    <h4>{selected.name}</h4>
                    <p>{selected.description}</p>
                  </div>
                  <button
                    type="button"
                    data-demo-target="close"
                    className={styles.iconButton}
                    aria-label="Close tool details"
                    onClick={() => interact({ selected: null })}
                  >
                    <Icon name="close" />
                  </button>
                </header>
                <ul className={styles.installations}>
                  {harnesses.map((harness) => (
                    <li key={harness}>
                      <HarnessMark harness={harness} />
                      <strong>{harnessNames[harness]}</strong>
                      {selected.harnesses.includes(harness) ? (
                        <code title={selected.origin}>{selected.origin}</code>
                      ) : isPortable(selected) ? (
                        <button type="button" onClick={() => install(selected, harness)}>
                          <Icon name="plus" />
                          Install
                        </button>
                      ) : (
                        <small>Not installed</small>
                      )}
                    </li>
                  ))}
                </ul>
                <div className={styles.tabs} role="tablist" aria-label="Tool information">
                  {(["file", "details"] as const).map((tab) => (
                    <button
                      id={`inventory-tab-${tab}`}
                      aria-controls="inventory-tool-content"
                      type="button"
                      role="tab"
                      data-demo-target={tab}
                      key={tab}
                      aria-selected={state.tab === tab}
                      onClick={() => interact({ tab })}
                    >
                      {tab === "file" ? "File" : "Details"}
                    </button>
                  ))}
                </div>
                <div
                  id="inventory-tool-content"
                  className={styles.panelBody}
                  role="tabpanel"
                  aria-labelledby={`inventory-tab-${state.tab}`}
                >
                  {state.tab === "file" ? (
                    <>
                      {selected.managed && (
                        <p className={styles.notice}>
                          Cutokyo manages this capture or search entry. Use Settings or uninstall to
                          change it.
                        </p>
                      )}
                      {["skill", "instruction"].includes(selected.kind) && (
                        <button
                          className={styles.editorButton}
                          type="button"
                          onClick={() => {
                            interact();
                            setExpanded(true);
                          }}
                        >
                          <Icon name="expand" />
                          Open editor
                        </button>
                      )}
                      <textarea
                        aria-label="Source content"
                        spellCheck={false}
                        readOnly={selected.managed}
                        value={fileContent}
                        onFocus={() => interact()}
                        onChange={(event) =>
                          setDrafts({ ...drafts, [selected.id]: event.target.value })
                        }
                      />
                    </>
                  ) : (
                    <div className={styles.details}>
                      <dl>
                        {[
                          ["Kind", kindNames[selected.kind]],
                          ["Scope", selected.scope],
                          [
                            "Format",
                            ["skill", "instruction"].includes(selected.kind) ? "markdown" : "json",
                          ],
                          ["Source", selected.origin],
                        ].map(([term, value]) => (
                          <div key={term}>
                            <dt>{term}</dt>
                            <dd>{value}</dd>
                          </div>
                        ))}
                      </dl>
                      {selected.kind === "skill" && (
                        <p>
                          Project scope: only this repository&apos;s agents load it. Supporting
                          files in the skill directory are copied with it.
                        </p>
                      )}
                      <h5>Where this came from</h5>
                      <dl>
                        <div>
                          <dt>Source</dt>
                          <dd>{harnessNames[selected.harnesses[0] ?? "claude_code"]}</dd>
                        </div>
                        <div>
                          <dt>Evidence</dt>
                          <dd>Local state</dd>
                        </div>
                      </dl>
                    </div>
                  )}
                </div>
                {!selected.managed && (
                  <footer className={styles.panelFooter}>
                    <button
                      type="button"
                      onClick={() => {
                        interact({ selected: null });
                        setTools((current) => current.filter((tool) => tool.id !== selected.id));
                      }}
                    >
                      Remove
                    </button>
                    <button
                      type="button"
                      disabled={fileContent === (saved[selected.id] ?? sourceContent(selected))}
                      onClick={() => {
                        interact();
                        setSaved({ ...saved, [selected.id]: fileContent });
                      }}
                    >
                      Save changes
                    </button>
                  </footer>
                )}
              </aside>
            )}
          </div>
        </div>
      </div>
      {automatic && visible && (
        <div
          className={styles.cursor}
          data-click={scene.click}
          style={{ transform: `translate(${cursor.x}px, ${cursor.y}px)` }}
          aria-hidden="true"
        >
          <span />
          <svg viewBox="0 0 24 28">
            <path
              d="M3 2v21l5.8-5.6 4.4 8.1 3.5-1.9-4.5-8.1 8-1.3L3 2Z"
              fill="#f5f5f5"
              stroke="#111"
              strokeWidth="1.5"
            />
          </svg>
        </div>
      )}
      <div className={styles.playback}>
        <span>
          <i />
          Interactive preview
        </span>
        <button
          type="button"
          aria-label={automatic ? "Pause inventory animation" : "Replay inventory animation"}
          onClick={() => {
            if (automatic) interact();
            else {
              setManual(initialState);
              setTools(inventoryTools);
              setEnabled(true);
              setTime(0);
              setReducedMotion(false);
              setPlaying(true);
            }
          }}
        >
          <Icon name={automatic ? "pause" : "play"} />
          {automatic ? "Pause" : "Replay"}
        </button>
      </div>
      {expanded && selected && (
        <div className={styles.editorOverlay}>
          <div role="dialog" aria-modal="false" aria-label={`Edit ${selected.name}`}>
            <header>
              <strong>{selected.name}</strong>
              <button
                type="button"
                autoFocus
                aria-label="Close editor"
                onClick={() => setExpanded(false)}
              >
                <Icon name="close" />
              </button>
            </header>
            <textarea
              aria-label="Expanded source content"
              value={fileContent}
              onChange={(event) => setDrafts({ ...drafts, [selected.id]: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === "Escape") setExpanded(false);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

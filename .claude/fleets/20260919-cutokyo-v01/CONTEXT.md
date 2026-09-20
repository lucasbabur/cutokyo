# Cutokyo v0.x — authoritative Fleet context

Fleet: `20260919-cutokyo-v01`  
Repository: `/home/lucas/Developer/personal/cutokyo-community`  
Remote: `https://github.com/lucasbabur/cutokyo` (public)  
Design date: 2026-09-19

This file resolves the product brief into one build contract. The original user
brief remains authoritative where this file is silent. English is the only product,
code, documentation, schema, fixture, issue, and release language.

## Outcome

Ship a genuinely usable pre-1.0 local-first application, not a polished scaffold.
A user installs one native Cutokyo CLI (optionally through npm) and/or the Tauri
desktop app, runs setup, sees Claude Code, Codex, and OpenCode coverage, captures
new activity without keeping Cutokyo open, drains it into one searchable SQLite
database, searches and resumes exact native sessions, understands usage and agent
infrastructure, controls shared MCP servers, verifies plugins, applies guards, and
can explicitly ask for AI summaries. The integrated artifact must be exercised as
a CLI, a browser-rendered frontend, and an actual Tauri app before acceptance.

All features in this contract are Apache-2.0 community features. There is no
enterprise implementation, proprietary service, account system, remote control
plane, organization/tenant model, cloud sync, or license service in this release.

## Scope and refusals

Cutokyo **is**:

- a local history, observability, search, resume, setup, and diagnostics product
  for Claude Code, Codex, and OpenCode;
- a durable evidence store for native events and transcripts with source coverage;
- a cross-platform Rust CLI and Tauri desktop app;
- a read-only agent search MCP plus an explicitly configured central MCP broker;
- a versioned out-of-process plugin host for sources and processors;
- an optional, consented proxy fallback and AI-analysis egress path;
- a place to show what is known, estimated, user-declared, conflicting, or absent.

Cutokyo **refuses to be**:

- a replacement coding harness or model provider;
- a cloud memory service, covert telemetry collector, or silent MITM;
- a second TypeScript CLI hidden behind npm;
- a tool that fabricates token, price, quota, context, or resume facts;
- a hard OS sandbox for arbitrary plugins when no portable sandbox is enforced;
- a phone app or enterprise tenancy platform in v0.x.

History and resume are in. Automatic prompt-context injection is out. The optional
search skill/MCP is read-only, user-disableable, and invoked by the agent or user;
it does not silently rewrite future prompts.

## Decisions that resolve contradictions

1. **Crates beat the prose sketch.** `cutokyo-domain` is a pure crate. `app`,
   `ingest`, `store`, and `adapters` are modules in `cutokyo-core`. CLI and desktop
   are separate binaries.
2. **Read-only means no direct DB writes.** CLI and desktop still invoke mutating
   app use cases for setup, config, analysis, resume, plugin execution, and MCP
   toggles. Only the core writer mutates SQLite.
3. **No always-on daemon in v0.x.** Desktop or CLI becomes the single core writer
   under a process lock and drains backlog at startup. Hooks/plugins only spool.
   A second frontend reports/connects to the owner rather than stealing the lock.
4. **One event per atomic rename.** Cross-platform multi-process append atomicity
   is not assumed. A hook creates one temporary file, flushes it, and atomically
   renames it into `spool/`; its content is one JSON line.
5. **Observation identity survives native bugs.** Prefer native event IDs. Where
   absent, canonicalize and fingerprint the raw payload. Session/event/sequence
   alone is insufficient because duplicate hooks and resume-ID drift are observed
   in real harnesses.
6. **Proxy is never a silent fallback.** Native sources may fall through
   automatically. Proxy activation requires explicit consent and remains visibly
   active. Instrumentation fails open so the harness works; an explicitly enabled
   outgoing secret guard blocks a channel it cannot safely inspect.
7. **Two MCP surfaces.** `cutokyo mcp` exposes Cutokyo read operations only. The
   central broker separately routes user-approved upstream MCP servers and updates
   harness config through reversible managed ownership.
8. **OpenAI harness means Codex.** Provider attribution remains a separate domain
   concept.
9. **npm is distribution.** cargo-dist generates an npm wrapper that downloads the
   correct native CLI. There is one implementation.
10. **Privacy is baseline engineering, not an enterprise feature.** Local-only and
    no telemetry are defaults; proxy/analysis require consent; keys use the OS
    keychain; bundles exclude content and full paths. Enterprise DLP and tenancy
    remain out.
11. **AI analysis has an egress contract.** Preview, redaction, exact payload scope,
    provider/model, confirmation, cancellation, retry, and idempotent writes are
    mandatory.
12. **Plugin side-effect boundaries are honest.** Plugins are subprocesses with no
    DB path/store handle. Transcript/network capabilities require approval. The
    project does not claim filesystem/network isolation it does not enforce.
13. **DCO applies to agents too.** Every commit has a matching `Signed-off-by` and
    the required Claude co-author trailer.
14. **Explicit compatibility contracts override the global clean-break default.**
    DB migrations are forward-only, historical spool parsers remain readable, and
    the public agent surface deprecates for one minor. No compatibility shims are
    kept for private implementation APIs.
15. **Config ownership follows the host format.** Marked text blocks are used where
    legal. Strict JSON is edited structurally with uniquely identifiable Cutokyo
    entries and byte-preserving backup/restore outside owned nodes. Before mutation,
    refuse symlinks/non-regular files, persist recovery intent, and recheck snapshots;
    absent managed state and repeated cleanup are successful no-ops.
16. **No migration or source donation from the predecessor.** This public repository
    starts a clean v0.x database and config namespace. The audited predecessor has
    no repository-wide license grant, so its implementation, schemas, tests,
    fixtures, prose, UI/CSS, images, fonts, workflows, and generated planning
    artifacts are not copied. Only generally described behavior and failure modes
    are independently restated in the clean-room ledger and reimplemented from this
    contract and public documentation. Old installs are not detected or rewritten.
17. **The local store is honest about at-rest protection.** v0.x redacts secrets
    before persistence, uses owner-only permissions, and recommends full-disk
    encryption, but does not claim application-level SQLite encryption. Adding
    SQLCipher later would be a new storage/key-recovery contract; plaintext local
    transcript retention is disclosed during onboarding.
18. **Settings have one contract.** Rust and TypeScript types are generated from one
    machine-readable schema, or UI commands use explicit patch semantics. A field
    omitted by one client cannot reset an unrelated guard or privacy option; unknown
    fields are rejected where exact contract handling is required.

## Predecessor clean-room decision

A read-only audit of `/home/lucas/Developer/personal/cutokyo` completed on
2026-09-19. It inspected source and declared tests but ran no build, test, installer,
provider call, migration, or release. The predecessor is a commercial proxy/control-
plane product with WorkOS tenancy, organization synchronization, managed webhooks,
and a license authority—not the architecture of this community application. No root
`LICENSE`, `COPYING`, or `NOTICE` granted reuse. Public visibility is not a license.

The audit is therefore a **failure-mode observation**, never a source donor. New
contributors must maintain:

- `docs/provenance/clean-room-policy.md` — what may and may not be consulted/copied;
- `docs/provenance/reference-observations.md` — general behavior observed, why it is
  needed, public corroboration, and a statement that no expression/fixture was copied;
- `docs/provenance/decision-ledger.md` — independently written requirement, new author,
  implementation/test locations, and reviewer approval;
- `THIRD_PARTY.md` — every shipped dependency/asset and its verified license.

### Salvage as independently worded behavior

| Behavior to preserve | Reference observation | New contract |
|---|---|---|
| loopback-only local services and explicit provider-host boundaries | predecessor `desktop/src/server.rs` and `desktop/src/capture/mod.rs` | bind locally, validate resolved addresses, and test exact allowlist edges |
| recovery intent before mutation; independent owned-subsystem rollback | predecessor `desktop/src/activation.rs` | durable plan, backup-once, snapshot recheck, managed ownership, no-state cleanup success |
| provider payload and persisted/telemetry payload are distinct | predecessor `desktop/src/security.rs` | redaction never mutates provider traffic unless a separately enabled outgoing guard does so |
| durable user-visible subsystem degradation | predecessor `desktop/src/observability.rs` | one persisted health projection shared by CLI, desktop, and doctor |
| quarantine before a sink/cursor acknowledges bad input | predecessor `desktop/src/observability.rs` | a rejected observation is durably quarantined before ingest advances past it |
| bounded files, protocol lines, payloads, queues, and diagnostic records | predecessor Rust and SDK modules | every external boundary has an explicit tested limit and actionable error |
| stable attributable session projections and idempotent ingest | predecessor `desktop/src/sessions.rs` and backend store | new native IDs/fingerprints, immutable evidence, rebuildable projections |
| non-overlapping polling and stale-request cancellation | predecessor local dashboard/model | app reads cannot race and overwrite newer project/session state |
| liveness and readiness are different | predecessor backend main module | local status may be alive while DB/spool/plugin readiness is degraded |
| packaged behavior must exercise the package, not a source binary | predecessor desktop artifact scripts | npm/Tauri/install smoke resolve and launch the built artifact itself |

### Adapt rather than copy

- Keep atomic spool + SQLite, not the predecessor's encrypted generation JSONL.
- Keep native hooks/APIs/OTel before proxy, not proxy-first interception.
- Keep the new major-handshake/capability/verifier plugin protocol, not the
  predecessor's partially validated `1.0` request/response hooks.
- Keep the independent graphite/warm-paper UI. The predecessor's monolithic React
  views, dark lime/cyan styling, charts, copy, logos, and tests are excluded.
- Keep one native Rust CLI with generated npm distribution. Generate help, structured
  output, UI defaults, and schemas from shared definitions so they cannot drift.
- Choose/redesign redaction rules from maintained libraries and synthetic/public
  corpora; do not copy predecessor regexes, entropy thresholds, placeholders, or
  fixtures.

### Discard

The commercial license service, WorkOS/organization product, managed webhook
plugins, provider compression/mutation implementation, existing schemas/SDKs/tests/
fixtures/UI/assets/fonts/marketing claims, private endpoint assumptions, legacy
memory deletion behavior, `.claude`/`.opencode` planning artifacts, bundled
`python-code-checker`, and stale release claims do not enter this repository.

### Failure modes promoted into gates

The predecessor's private `EventStoreHealth` used a separate persisted JSON document
and in-memory mutex. We preserve no code or schema from it. Its useful lesson is that
health itself can fail and must survive restart. The new bounded persisted projection
must distinguish current state from lifetime history and include:

- current degraded state, last persistence success, last failure time, and sanitized
  failure category;
- current and lifetime quarantine counts plus first currently affected observation;
- health-persistence status, spool cap/drain state, writer lock, schema/derive version,
  last integrity check, projection rebuild, and backup/restore verification;
- independent error dimensions, so one successful write cannot clear retention,
  quarantine, backup, or integrity failures;
- identical snapshot semantics in CLI JSON, desktop, and doctor without scanning all
  historical events during ordinary status reads.

The predecessor also exposed an uninstall defect: emergency restore errored when no
activation state existed, while NSIS aborted uninstall on any restore failure. The
new release blockers explicitly cover install-never-activated, missing/empty state,
repeated restore, partially corrupt state, concurrent user edits, and independent
file/proxy/trust cleanup failures. “Nothing is managed” is a successful idempotent
cleanup, not an error.

Other promoted checks: runtime validation and size bounds on every plugin message;
one generated/patch settings contract so omitted fields cannot reset unrelated
controls; exact UI language separating telemetry redaction from provider-bound
protection; backup digest verification before destructive restore; symlink/non-regular-
file refusal; build never rewrites tracked source; readiness is not liveness; and
missing signing material blocks publication rather than emitting an unsigned release.

## Repository and runtime boundaries

```text
cutokyo-cli                cutokyo-desktop
     └──────────────┬──────────────┘
                    ▼
             cutokyo-core::app
          ┌─────────┼───────────┐
          ▼         ▼           ▼
       ingest      store      adapters
          └─────────┼───────────┘
                    ▼
             cutokyo-domain
        no fs · no http · no sql · no process
```

Runtime:

```text
Claude Code hooks ─┐
Codex native tiers ├──► atomic spool files ──► core writer ──► SQLite
OpenCode plugin ───┘                              ▲               │
local API · OTel · CLI · state files ─────────────┘               │
                                                                  ▼
                           desktop · CLI · cutokyo MCP ──► app/read ports
```

Only `app` wires siblings. Adapters produce immutable raw observations; resolvers
turn them into projections. UI/CLI/MCP never import store internals.

## Domain and truth model

The minimum model is Harness → Account/Project → Session → Turn → Message →
ToolCall/AgentRun, with Summary written later. InstallationSnapshot contains
ConfigItem (`mcp | skill | hook | plugin`, state, scope, origin). RawObservation,
Coverage, ContextBreakdown, billing basis, PriceSnapshot, Account, and QuotaWindow
remain first-class because dashboards cannot be truthful without provenance and
validity intervals.

Every normalized fact links its raw observation and records source tier, capture
time, native identity, parser version, and confidence/coverage. A projection is
rebuildable. A raw observation is immutable while retained; explicit session or
history deletion removes its raw evidence, projections, FTS rows, and summaries in
one core transaction. Unknown never becomes zero. A
user-declared plan stays user-declared. Conflicting sources resolve through a
visible precedence rule and never double count.

## Capture precedence and fallback

| Priority | Channel | Nature |
|---:|---|---|
| 1 | hook/plugin event | pushed, real-time, native identity |
| 2 | documented local API/app server | pulled, structured |
| 3 | OpenTelemetry | pushed usage and attribution |
| 4 | harness CLI | pulled, structured, process cost |
| 5 | local state/transcripts/config | complete but unstable internal formats |
| 6 | file watch | trigger only; rereads tier 5 |
| 7 | provider usage API | authoritative coarse spend, credentials |
| 8 | proxy | full request/header fidelity, explicit consent |
| 9 | TUI scrape | labelled last resort |
| 10 | user declaration | legitimate, permanently labelled |

| Fact | Preferred source | Fallback |
|---|---|---|
| token usage | OTel/local API | proxy |
| sessions/transcript | local API/state | none |
| exact resume target | native state/API | none |
| tool/skill/agent events | hooks | state |
| quota remaining | CLI/headers | labelled learned ceiling |
| inventory | config + CLI | none |
| context breakdown | proxy | unavailable |

Coverage is a product surface. An adapter encountering an unknown version preserves
raw input, reports reduced coverage, and does not silently apply an old parser.

## Storage and spool

- SQLite resides on a local filesystem and uses WAL, 5-second busy timeout on every
  connection, `synchronous=NORMAL`, and foreign keys.
- One writer is guarded by a process lock. Reads cannot hold transactions across
  user interaction or network work.
- Online backup API or `VACUUM INTO` is used; a live `.db` file is never copied
  alone. Every backup carries a digest. Restore verifies the digest before replacing
  current state, runs with all connections closed, and finishes with integrity
  checking; failure cannot strand the application stopped or erase the prior copy.
- The database holds a bounded, restart-safe subsystem-health projection. Ordinary
  status reads never scan full history; CLI JSON, desktop, and doctor consume the
  same snapshot and distinguish current from lifetime quarantine.
- FTS5 capability and the bundled SQLite version are asserted in tests and doctor.
- Each spool filename is unique and created through temp-file + atomic rename.
  `observation_id` is unique in SQLite, so replay is `INSERT OR IGNORE` safe.
- Malformed input moves to quarantine with a counted health record. A broken line
  never crashes the drain.
- Spool caps are 30 days or 500 MB. At the cap, new capture stops visibly; Cutokyo
  never silently discards history. This backlog cap is not the database retention
  policy.
- Stored history defaults to keep-until-deleted, with visible configurable retention,
  one-session deletion, and delete-all preview/confirmation. Deletion covers raw,
  derived, FTS, and summary rows transactionally and never promises physical secure
  erasure from SSDs or existing backups; the product explains checkpoint/vacuum and
  backup implications honestly.
- Migrations are forward-only. Older binaries refuse newer schemas. Derived tables
  rebuild on `derive_version`. Every released spool format remains readable.

## Product surfaces

### CLI

Human and stable `--json` modes cover setup/dry-run/uninstall, sessions/search/show/
delete, resume, retention preview/apply, config/origins, hook/drain, plugins, MCP,
analyze, doctor, bundle, backup, and version. Exit codes distinguish usage, unavailable capability, invalid
contract, unhealthy state, and internal failure.

### Desktop

The Tauri app covers onboarding/coverage, dashboard, searchable session history,
session detail/resume/delete, retention and delete-all controls, installed hooks/
skills/plugins/MCPs, MCP control, plugin health, guard/proxy coverage, analysis
consent/progress, quotas/prices/unknowns, spool/quarantine/lock health, doctor/
bundle, settings, and updater choice.

Visual direction: a calm local observability console—graphite and warm-paper
surfaces, crisp neutral type, vermilion only for live capture and actionable
warnings, compact information density, no terminal cosplay and no generic purple
SaaS gradients. Required windows are 1280×800, 1536×960, and a 900×700 minimum.

### Plugins

Source plugins receive harness identity, capabilities and a cursor/window; output
immutable raw observations and must be cursor-idempotent. Processor plugins receive
normalized records and optional approved transcripts; output tags/derived facts or
nothing. JSON-line subprocess messages have a major-version handshake, bounded
size, cancellation, timeout, and structured diagnostics. TypeScript processor and
Python source examples must pass `cutokyo plugin verify`, whose mutation fixtures
must fail.

### MCP

The read-only Cutokyo server exposes session/history/usage/coverage/inventory
queries. The central broker routes enabled stdio and streamable-HTTP upstreams,
namespaces tools, contains one upstream failure, and gives every harness the same
approved set. Setup/uninstall restores host configuration exactly outside owned
entries.

### Guards, proxy, and analysis

Synthetic-secret tests cover headers, URLs, nested JSON, tool output, multiline and
split chunks, plus benign entropy. No raw finding crosses the logging boundary.
Proxy binds locally, never persists credentials, requires consent, stays visible,
and fails open for harness operation. AI analysis uses a fake provider for required
acceptance; a real credential dry run is additional evidence, not a hidden blocker.

## Quality and evidence

Blocking Rust checks: format, clippy warnings, tests, cargo-deny licenses/bans,
domain purity, architecture/upgrade/pipeline/E2E, and typos. Advisories,
cargo-pup, and cargo-machete report warnings initially. Blocking TypeScript checks:
no-emit, Oxlint, Prettier, Vitest, browser E2E and accessibility; Knip warns until
clean. Fast lefthook checks only changed files.

CI lints once on Linux, then builds/tests Linux, macOS, and Windows. This
non-publishing Fleet validates those remote job contracts and runs the real Linux
artifact, but does not claim Windows/macOS jobs or artifacts ran; their first real
success is a separate release gate after an authorized push/tag. Fake harnesses are
deterministic and credential-free. Scheduled drift probes genuinely installed
public harness versions and opens an issue without gating PRs. Native desktop QA
uses WebdriverIO's Tauri service; browser tests do not count as native evidence.

A dedicated Haiku checker also loads the installed `jev-qa` skill, validates one short
YAML batch, and runs eight isolated browser cases per verification round. It preserves
JEV's PASS/ERROR/BLOCKED semantics and reads every generated screenshot before
reporting. Missing or unread evidence blocks acceptance. This is an independent,
inexpensive interaction/visual signal—not a substitute for native Tauri, package,
storage, or installed-harness proof.

The root `tests/` directory is the `cutokyo-integration-tests` workspace package;
its Cargo manifest declares the root-level `pipeline.rs`, `e2e.rs`, `upgrade.rs`,
and `architecture.rs` targets explicitly, so acceptance commands execute rather
than leaving decorative test files outside Cargo's graph.

Every gate selftest proves a complete known-good fixture passes and one targeted
known-bad mutation fails per gate. Exact missing-root CLI exit behavior is not part of
the acceptance contract. Gate selftests run before results are trusted. Diffguard scans added lines for
suppressions, skipped/focused tests, unsafe casts, threshold weakening, and fixture
substitution.

## Distribution

cargo-dist builds the native CLI, checksums, installers and npm wrapper. Tauri
builds MSI/NSIS, DMG, AppImage and `.deb`. Release workflows attach CycloneDX SBOM,
SHA-256 checksums and GitHub build provenance. Tauri updater manifests are signed;
platform code signing/notarization remain separately documented. This Fleet
performs package/install and release-plan dry runs only. It does not publish a tag,
release, npm package, or updater.

## Versioning

| Surface | Scheme | Rule |
|---|---|---|
| app | tag semver | remain 0.x until contracts stabilize |
| DB | forward-only integer | new migrates old; old refuses new |
| derived data | integer | bump rebuilds projections |
| spool | integer | all released readers remain |
| plugin protocol | major integer | reject unknown major at handshake |
| MCP/structured CLI | app semver | deprecate for one minor |

## Fleet construction order

1. Contract architect creates the first committed foundation and verifier selftests.
2. Core builder proves fake observation → spool → SQLite/FTS → search.
3. Architect merges core so all feature worktrees share real ports.
4. Claude, Codex, OpenCode, extensions, CLI/ops, and desktop build in parallel.
5. Architect merges all branches and runs integrated smoke checks.
6. Gatekeeper, product QA, and the Haiku JEV checker independently run command,
   native/runtime, and supplemental browser evidence.
7. Technical reviewer judges every criterion from artifacts, not summaries, and
   never treats JEV browser output as native proof.
8. Verified product defects return to owners; repairs merge and rerun affected checks,
   at most three attempts for the same blocker. A demonstrably invalid check is never
   silently weakened or repaired during the run: the workflow stops and calls the
   human with the contradiction and evidence.
9. Only a green integrated revision receives a final evidence report and rendered
   review. Nothing is pushed, published, or released by this workflow.

## Environment established before dispatch

- The new public remote exists and the local repository had no product commit when
  the design began.
- Installed locally: Rust 1.98.1, Cargo 1.98.1, Node 22.22.3, npm 10.9.8,
  Claude Code 2.1.277, Codex CLI 0.153.4, and OpenCode 1.18.28.
- `cargo-tauri`, `tauri-driver`, `WebKitWebDriver`, and `xvfb-run` were not on PATH
  during design. Builders may install trusted project/dev dependencies; QA must
  distinguish unavailable infrastructure from product failure.
- Chrome is installed for browser-mode journeys.
- No Herdr SSH machine is saved, and the design-time read-only probe of
  `lucas@lucasmasterblasterserver` timed out. The workflow therefore has no Hermes
  or remote macOS/Windows escape hatch; unavailable native infrastructure must be
  reported as evidence gaps rather than silently replaced with browser/Linux proof.
- `cutokyo-gates` is registered as
  `20260919-cutokyo-v01/cutokyo-gates`; its selftest passes and the empty repository
  fails all product gates. Missing-root CLI exit behavior is not an acceptance gate.
- Reused diffguard: `20260907-acquirer-routing/diffguard`.

## Research-backed failure modes

The Fleet tests known ecosystem failures instead of trusting happy-path docs:

- Tauri can render correctly in a browser yet show blank native windows on Linux,
  Windows, or macOS; native package smoke and screenshots are mandatory.
- SQLite WAL still has one writer, can return `SQLITE_BUSY`, can starve checkpoints,
  and cannot be safely copied or placed on network storage.
- Claude Code hooks have reported duplicate SessionStart delivery and resume ID
  misattribution; its transcript format is internal and mutable.
- Codex app-server resume has reported incomplete/paginated histories and dropped
  events; exact native identifiers and completeness checks are mandatory.
- OpenCode plugin lifecycle events can be unawaited or change across plugin API
  generations; finalization and drift fixtures are mandatory.
- Users want durable local cross-tool search but fear plaintext secrets, client
  context, wrong/abandoned attempts, and silent transcript deletion. Local storage
  does not remove the need for retention, redaction, deletion, and bundle safety.

Primary references:

- Tauri capabilities, updater, distribution and native testing:
  <https://tauri.app/security/capabilities/>,
  <https://v2.tauri.app/plugin/updater/>,
  <https://v2.tauri.app/distribute/>,
  <https://v2.tauri.app/develop/tests/webdriver/>.
- SQLite WAL, transactions, backup, FTS5 and network guidance:
  <https://www.sqlite.org/wal.html>,
  <https://www.sqlite.org/lang_transaction.html>,
  <https://www.sqlite.org/backup.html>,
  <https://www.sqlite.org/fts5.html>,
  <https://www.sqlite.org/useovernet.html>.
- Claude Code hooks, OTel, and transcript storage:
  <https://code.claude.com/docs/en/hooks>,
  <https://code.claude.com/docs/en/monitoring-usage>,
  <https://code.claude.com/docs/en/claude-directory>.
- Codex app server, configuration and CLI resume:
  <https://developers.openai.com/codex/app-server>,
  <https://developers.openai.com/codex/config-advanced>,
  <https://developers.openai.com/codex/cli>.
- OpenCode plugins and SDK:
  <https://opencode.ai/v2/docs/build/plugins>,
  <https://opencode.ai/v2/docs/build/sdk>.
- Official Rust MCP SDK: <https://github.com/modelcontextprotocol/rust-sdk>.
- cargo-dist npm installer: <https://axodotdev.github.io/cargo-dist/book/installers/npm.html>.
- DCO 1.1: <https://developercertificate.org/>.

Relevant reported regressions are preserved as fixture requirements, not treated as
permanent upstream behavior: Tauri blank windows
(<https://github.com/tauri-apps/tauri/issues/15050>), Claude duplicate resume hooks
(<https://github.com/anthropics/claude-code/issues/23932>), Codex incomplete resume
history (<https://github.com/openai/codex/issues/37577>), and OpenCode plugin
finalization timing (<https://github.com/anomalyco/opencode/issues/16879>).

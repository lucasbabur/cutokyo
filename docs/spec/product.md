# Cutokyo v0.x product specification

Status: approved pre-1.0 community contract, 2026-09-19.

## Contents

1. [Purpose and scope](#purpose-and-scope)
2. [Non-goals](#non-goals)
3. [Architecture and ownership](#architecture-and-ownership)
4. [Truth and provenance](#truth-and-provenance)
5. [Capture precedence](#capture-precedence)
6. [Storage, spool, and health](#storage-spool-and-health)
7. [Search, resume, and deletion](#search-resume-and-deletion)
8. [Configuration mutation](#configuration-mutation)
9. [Plugins and MCP](#plugins-and-mcp)
10. [Guards, proxy, and analysis](#guards-proxy-and-analysis)
11. [Product surfaces](#product-surfaces)
12. [Quality, distribution, and versions](#quality-distribution-and-versions)

## Purpose and scope

Cutokyo is a local-only, local-first community application for Claude Code,
Codex, and OpenCode. It preserves attributable session history, searches across
harnesses, resumes exact native sessions, explains usage and coverage, inventories
agent infrastructure, and provides reversible controls around plugins, MCP, guards,
proxy capture, and user-requested analysis.

All specified v0.x features are Apache-2.0 community features. There is no account,
organization, tenant, hosted storage, remote control plane, proprietary extension,
license server, or default telemetry path.

The product is pre-1.0. Private implementation APIs may change without compatibility
shims. The explicit persisted and public version contracts below are exceptions.

## Non-goals

Cutokyo is not a replacement coding harness, model provider, cloud memory service,
silent context injector, or invisible MITM. It does not fabricate token usage,
quota, price, context size, or resume identity. It does not claim a portable hard
sandbox for arbitrary subprocess plugins. It does not ship enterprise tenancy or
mobile applications in v0.x.

History and exact resume are in scope. Automatic prompt-context injection is out of
scope. The optional search MCP is read-only and user-disableable.

## Architecture and ownership

`cutokyo-domain` is a pure Rust crate containing IDs, values, provenance rules,
settings patch semantics, errors, and dependency-inversion ports. It has no
filesystem, network, SQL, process, Tauri, or async-runtime capability.

`cutokyo-core` contains the sibling modules `app`, `ingest`, `store`, and `adapters`.
Only `app` composes siblings. CLI and desktop frontends invoke application use cases;
they do not open SQLite or write spool internals. Hooks and external plugins never
receive a store handle or database path.

There is no always-on daemon in v0.x. One CLI or desktop core process acquires the
writer lock and drains backlog. A second frontend reports or connects to the owner;
it never steals the lock or guesses that the owner died.

The central error vocabulary distinguishes invalid input, invalid contract,
unsupported protocol major, unavailable capability, writer ownership, capacity,
not found, unhealthy state, cancellation, and internal failure. JSON errors include
safe field/expected/actual context and never echo raw secrets.

## Truth and provenance

Raw observations are immutable evidence while retained. Derived projections are
rebuildable. Every normalized fact links one or more raw observation IDs and keeps:

- capture channel and fixed precedence tier;
- capture timestamp;
- native event, session-tree, and exact resume identities when available;
- parser and derivation versions;
- observed, estimated, user-declared, conflicting, or unknown confidence;
- complete, partial, disabled, unavailable, or unknown-version coverage.

Unknown is not zero, false, an empty list, or an estimate. User declarations remain
labelled. Conflicting sources select one visible winner by precedence and retain all
raw evidence without double counting.

Observation identity prefers a native event ID. Without one, adapters canonicalize
and fingerprint the raw payload. Session/event/sequence alone is not sufficient
because duplicate native events and resume-ID drift are expected.

## Capture precedence

Facts use the first channel that can establish them:

1. hook or plugin event;
2. documented local API or app server;
3. OpenTelemetry;
4. structured harness CLI output;
5. local state, transcripts, or configuration;
6. file watch as a trigger to reread local state;
7. provider usage API;
8. proxy, only after explicit consent;
9. labelled TUI scraping;
10. labelled user declaration.

Native fallthrough may be automatic. Proxy activation is never automatic: it must
follow an explicit consent use case and retain a persistent, visible active state.
An unknown source version preserves raw input, reports reduced coverage, and never
silently invokes an old parser.

## Storage, spool, and health

Each capture writer creates one unique temporary file, writes exactly one JSON line,
flushes it, and atomically renames it into `spool/`. Concurrent JSONL append is not
assumed atomic across platforms. A unique observation ID makes replay idempotent.
Malformed or truncated input is durably quarantined before its cursor advances, and
the drain continues.

Spool retention is capped at 30 days and 500 MB. At either cap, capture visibly
refuses new observations instead of silently deleting old ones. This cap is separate
from database history retention.

SQLite is local disk only. Every connection enables WAL, foreign keys,
`synchronous=NORMAL`, and a five-second busy timeout. Transactions stay short. One
process lock owns all writes. Tests must assert the bundled SQLite version and FTS5.

Backups use SQLite's online backup API or `VACUUM INTO`, carry a digest, and are
verified before destructive restore. Restore runs with connections closed, retains
the prior copy until integrity succeeds, and cannot strand the application stopped.
Copying a live `.db` file alone is forbidden.

Health is a bounded persisted projection shared byte-for-field by CLI, desktop, and
doctor. It records current degradation, last successful health persistence, last
failure time and sanitized category, current and lifetime quarantine, first affected
observation, spool cap/drain, writer lock, schema/derive state, integrity/rebuild,
and backup/restore. Success in one dimension cannot clear another. Ordinary health
reads never scan full history.

v0.x uses owner-only database and spool permissions and recommends full-disk
encryption. It does not claim application-level encryption.

## Search, resume, and deletion

Search supports content, project, branch, harness, date, tool, skill, and agent
filters. Results expose source and uncertainty. Resume always launches the recorded
native target for the selected session. In particular, native session-tree identity
must not be substituted for a distinct resume identity.

History defaults to keep-until-deleted. Retention changes are previewed before apply.
One-session deletion and confirmed delete-all remove linked raw evidence,
projections, FTS rows, and summaries in one core transaction while preserving
unrelated sessions.

The product states that logical deletion does not physically erase SSD blocks, WAL
pages, or prior backups. It explains checkpoint, vacuum, and backup implications
without promising secure erasure.

## Configuration mutation

Read-only frontend architecture means no direct database writes, not no actions.
Setup, resume, configuration, plugin runs, MCP toggles, and analysis are app use
cases.

Before the first external mutation, setup persists recovery intent. It backs up a
regular configuration file once, records and rechecks its snapshot, preserves
unmanaged bytes or structures and permissions, and writes atomically. Symlinks and
non-regular targets are refused. Strict JSON receives structural edits rather than
comments; marked blocks are used only in formats that allow them.

Uninstall removes only uniquely owned Cutokyo entries. Missing or empty managed state
and repeated restore are successful no-ops. Corrupt or partial state, concurrent user
edits, and independent subsystem cleanup failures remain explicit recovery cases.

Secrets live in the OS keychain, never settings TOML or logs. One JSON Schema owns
settings fields. Frontend writes are omission-preserving patches, reject unknown
fields, and cannot reset unrelated privacy or guard controls.

## Plugins and MCP

External source and processor plugins are versioned JSON-line subprocesses. A major
integer handshake rejects unknown majors. Hosts validate every inbound and outbound
message at runtime and enforce line, message, field, telemetry, output, queue,
timeout, and cancellation bounds. A malformed plugin run cannot poison a later run.

Source plugins receive harness identity, approved capabilities, and a cursor/window;
they emit immutable, cursor-idempotent observations. Processor plugins receive
normalized records and optional approved transcripts; they emit tags, derived facts,
or nothing. Transcript and network access require declared and approved capabilities.
Plugins receive messages, never a store handle or path. Cutokyo does not claim OS
filesystem or network isolation where none is enforced.

`cutokyo mcp` is a read-only search and inspection server. A separate central broker
routes explicitly configured stdio and streamable-HTTP upstreams, namespaces tools,
contains one upstream failure, synchronizes enabled state across harnesses, and
restores only owned config on uninstall.

## Guards, proxy, and analysis

Secret detection is projected to sanitized finding types before logging. Synthetic
tests cover headers, URLs, nested JSON, tool output, multiline and split chunks, plus
benign high-entropy controls. Diagnostic bundles contain no prompts, transcripts,
raw secrets, or full project paths.

Instrumentation failure degrades open so the coding harness continues. A separately
enabled outgoing secret guard blocks a channel it cannot safely inspect rather than
claiming protection. Coverage says inspected, disabled, or unavailable; unavailable
never appears as zero findings.

Proxy capture binds locally, never persists credentials, requires explicit consent,
remains visibly active, and is used only for facts unavailable from native sources.
Without proxy coverage, context breakdown is unavailable rather than a row of zeros.

AI analysis begins with a preview naming the sessions and content scope, redaction,
provider, and model. The user can confirm, cancel, and retry. A confirmed request is
redacted before egress and stores provider, model, prompt version, source session IDs,
time, coverage, and idempotency key with its summary. Cancel sends no request; retry
does not duplicate summaries.

## Product surfaces

The native CLI will provide human and stable JSON modes for setup and uninstall,
sessions and search, show/delete/retention, resume, config origins, hooks and drain,
plugins, MCP, analysis, doctor and bundle, backup, and version. Exit codes distinguish
usage, unavailable capability, invalid contract, unhealthy state, and internal
failure.

The Tauri desktop application will cover onboarding and coverage, dashboards,
history/detail/resume/delete, retention/delete-all, inventory and MCP control, plugin
and guard health, proxy and analysis consent, quotas/prices/unknowns, spool and writer
health, doctor/bundle, settings, and updater choice. Native Tauri evidence is required;
browser mode cannot prove a native window.

The visual direction is a compact, accessible local observability console using
graphite and warm-paper surfaces, neutral type, and vermilion only for live capture
or actionable warnings. Required windows are 1280×800, 1536×960, and 900×700.

## Quality, distribution, and versions

Blocking Rust checks are formatting, Clippy with warnings denied, workspace tests,
cargo-deny licenses/bans, architecture, upgrade, pipeline/E2E, and typos. Blocking
TypeScript checks are no-emit type checking, Oxlint, Prettier, Vitest, browser E2E,
and accessibility. Advisory and unused-dependency tools initially report warnings.
Every static gate first proves a complete good fixture and one targeted bad mutation.

CI lints once on Linux and builds/tests Linux, macOS, and Windows. Local Linux success
is not evidence that remote artifacts ran. Scheduled drift installs genuine public
harness versions and may open an issue; it is not a required pull-request check.

cargo-dist builds one native CLI plus an npm downloader wrapper. Tauri builds native
bundles. Release plans include checksums, CycloneDX SBOM, and build provenance. This
implementation workflow does not push, tag, publish, or release. Missing signing
material blocks publication rather than producing an unsigned release.

Version surfaces are independent:

| Surface | Scheme | Rule |
| --- | --- | --- |
| Application | release-tag semantic version | stay 0.x until contracts stabilize |
| Database | forward-only integer | new binaries migrate old; old binaries refuse newer |
| Derived projections | integer | rebuild when changed; do not migrate derived rows |
| Spool | integer | readers for every released format remain available |
| Plugin protocol | major integer handshake | reject unknown majors clearly |
| Structured CLI and agent MCP | app semantic version | one-minor deprecation window |

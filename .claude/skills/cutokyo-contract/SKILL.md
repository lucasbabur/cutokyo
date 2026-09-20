---
name: cutokyo-contract
description: Cutokyo's architectural, capture, storage, versioning, and extension invariants. Load before changing domain, ingest, adapters, plugins, MCP, config, or persistence.
---

# Cutokyo contract

Cutokyo is a local-first session observability and control surface for Claude Code,
Codex, and OpenCode. It preserves searchable history, can resume native sessions,
shows usage and installed agent infrastructure, and exposes optional local agent
search. It is not a cloud account system, a replacement harness, or an invisible
MITM.

## Clean-room boundary

The predecessor at `/home/lucas/Developer/personal/cutokyo` has no repository-wide
license grant. Do not read or copy its code, schemas, tests, fixtures, prose, UI,
assets, workflows, or agent scaffolding. The only usable knowledge is the general,
independently worded behavior and failure-mode ledger in Fleet context. Record new
implementation/test provenance and every third-party license before shipping.

## Boundaries

- `cutokyo-domain` is a pure crate: values, provenance, rules, and ports only. No
  filesystem, network, SQL, process execution, desktop shell, or async runtime.
- `cutokyo-core` contains `app`, `ingest`, `store`, and `adapters`. Only `app`
  wires siblings. CLI and desktop call use cases; they never write SQLite directly.
- Hooks and plugins never open SQLite. They write atomic spool entries. A desktop
  or CLI core instance drains them while holding the single-writer process lock.
- A second frontend may read or connect through the owning core; it never steals
  the write lock or guesses that another writer is dead.
- External plugins are versioned subprocesses. They receive messages, never a DB
  path or store handle. Network and transcript access are declared capabilities
  requiring approval.

## Truth and provenance

Raw observations are immutable evidence. Projections may be rebuilt. Every
normalized fact carries source tier, capture time, native identity, parser
version, confidence/coverage, and raw observation linkage. Never collapse
"unknown" into zero, false, or an invented estimate.

Use native channels in this order when they can establish the fact: hooks/plugin
events, documented local API, OTel, CLI output, local state, file watch as a
trigger, provider usage API. Proxy capture is an explicit opt-in fallback with a
persistent visible status. TUI scraping is a labelled last resort. User-declared
facts stay labelled as declarations.

Duplicate native hooks and session-ID drift are expected. Prefer native event IDs.
Otherwise derive a stable identity from a canonical payload fingerprint while
keeping the original payload. The database enforces idempotency with a unique
observation ID.

## Spool and SQLite

Write one event to a create-new temporary file, flush it, then atomically rename
it into `spool/`; the file is one valid JSON line. This is safer cross-platform
than assuming concurrent append writes cannot interleave. Quarantine malformed or
truncated entries durably before advancing their ingest cursor, then continue. Cap
retained spool data by both age and bytes;
stop accepting new observations visibly rather than silently dropping old ones.

SQLite is local disk only, with WAL, `foreign_keys=ON`, `synchronous=NORMAL`, and
a 5-second busy timeout on every connection. Keep write transactions short. Use
SQLite's online backup API, never copy a live `.db` alone. Digest backups and verify
before destructive restore; retain the prior copy until integrity succeeds. Test the
bundled SQLite version and FTS5 capability. New binaries migrate old schemas forward;
old binaries refuse newer schemas. Derived data is rebuilt when `derive_version`
changes.

Health is a bounded persisted projection, not an in-memory flag or ordinary full
history scan. It distinguishes current/lifetime quarantine, health persistence,
spool cap/drain, writer lock, schema/derive, integrity/rebuild, and backup/restore;
CLI, desktop, and doctor use the same snapshot. A success in one dimension never
clears another failure. Stored history defaults to keep-until-deleted and supports
previewed retention, one-session deletion, and confirmed delete-all across raw,
derived, FTS, and summary rows. Do not promise physical SSD/WAL/backup erasure.
Database/spool permissions are owner-only; v0.x discloses that it is not application-
encrypted and recommends full-disk encryption.

## Versioned surfaces

- App: semantic version from the release tag.
- DB: forward-only integer with downgrade refusal.
- Derived projections: integer; rebuild, do not migrate.
- Spool: integer; readers for every released format remain forever.
- Plugin protocol: major integer handshake; reject unknown majors clearly.
- Agent surface (`cutokyo mcp`, structured CLI output): app semver, with one-minor
  deprecation because this explicit public contract overrides the general
  no-compatibility default.

Every JSON Schema is draft 2020-12, has a stable `$id`, and ships with golden good
and bad fixtures. Validate every plugin message in both directions at runtime, with
explicit line, message, field, telemetry, and output bounds plus malformed-output
recovery, cancellation, and timeouts. Protocol errors identify the message, field,
expected value, and actual value without exposing secrets. Never claim portable
filesystem or network sandboxing unless the platform actually enforces it.

## User-controlled mutation

"Read-only UI/CLI" means no direct database writes, not no actions. Setup, resume,
analysis, configuration, plugin runs, and MCP toggles are application use cases.
Before the first external mutation, setup persists recovery intent. It backs up a
regular configuration file once, rechecks the snapshot before an atomic write,
preserves unmanaged bytes/structure and permissions, and refuses symlink or
non-regular targets. Uninstall removes only Cutokyo-owned entries. Missing or empty
managed state is a successful idempotent no-op; corrupt/partial state, repeated
restore, concurrent user edits, and independent subsystem cleanup are explicit
recovery cases.

One canonical machine-readable settings contract owns Rust and TypeScript types or
commands use patch semantics. Omitted UI fields must never reset unrelated settings;
reject unknown fields where the public contract requires exactness.

The Cutokyo search MCP is read-only. The central MCP broker is separate: it routes
explicitly configured upstream servers, exposes enable/disable state across
harnesses, contains one upstream failure, and restores harness config on uninstall.

## Egress and guards

Local-only and no telemetry are defaults. Before AI analysis or proxy capture,
show what content leaves the machine, which provider/model receives it, and what
redaction ran; require explicit confirmation and support cancel/retry. Store the
provider, model, prompt version, source sessions, and idempotency key with a
summary.

Project all detections into sanitized types before logging. Diagnostic bundles
contain no prompts, transcripts, raw secrets, or full project paths. Instrumentation
failure degrades open so the harness keeps working; an explicitly enabled outgoing
secret guard blocks rather than claiming protection it could not apply. Coverage
must say where each guard is and is not active. UI copy keeps telemetry/on-disk/
bundle redaction distinct from provider-bound inspection or mutation; an unavailable
channel is unknown coverage, never "zero findings."

## Change discipline

Use maintained libraries after checking release recency and adoption. Keep the
implementation simple, but never replace a required behavior with a scaffold.
Run `cutokyo-gates selftest` before trusting the gate, then the applicable Rust,
TypeScript, fixture, integration, and E2E checks. Do not weaken gates, add
suppressions, skip tests, or substitute hand-authored evidence. Make atomic commits
with both `Signed-off-by` and the required Claude co-author trailer.

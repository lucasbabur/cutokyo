---
name: cutokyo-runtime-qa
description: Run Cutokyo as a real local product against fake and installed harnesses, verify desktop and CLI journeys, and collect reproducible acceptance evidence.
---

# Cutokyo runtime QA

Use this only against the integrated revision. Builder reports are context, not
evidence. A missing executable, package, fixture, screenshot, or credential makes
the affected verdict blocked or failed; it never becomes a pass.

## Establish the test environment

Record the revision, OS, architecture, Rust/Node/npm versions, bundled SQLite
version, and Claude Code/Codex/OpenCode versions. Use an isolated temporary
`CUTOKYO_CONFIG_HOME` and `CUTOKYO_DATA_HOME`. Never point a destructive setup,
uninstall, migration, or fixture command at the operator's real home directory.

Run in this order:

1. `cutokyo-gates selftest`, then `cutokyo-gates all`.
2. The repository's format, clippy, tests, dependency, TypeScript, and fixture gates.
3. The fake-harness E2E suite, including every released spool format.
4. CLI journeys against temporary state.
5. React/Vite browser E2E and accessibility checks.
6. Supplemental Haiku JEV batch via the installed `jev-qa` skill: validate the YAML,
   run the exact workflow cases once in isolated sessions, and read every screenshot.
   ERROR, BLOCKED, missing evidence, or unread screenshots fail JEV acceptance.
7. Actual packaged Tauri smoke/E2E using WebdriverIO's Tauri service where the
   platform supports it; JEV/browser output is not a substitute.
8. Read-only or dry-run probes of installed harnesses. Never send a paid model
   request merely to prove capture unless the workflow explicitly authorizes it.
9. Packaging/install dry runs from built artifacts, not source shortcuts. Snapshot
   tracked files before and after: a build that rewrites source fails.

## Required adversarial worlds

### Ingest and storage

Exercise duplicate hook delivery, hook session-ID drift, out-of-order events,
unknown parser versions, a truncated spool file, month-old backlog, byte and age
caps, concurrent readers with one writer, a second writer, long reads around a
checkpoint, migration from every fixture DB, downgrade refusal, online backup,
tampered-backup refusal before replacement, failed-restore recovery, FTS5 search,
persisted-health corruption/restart, current versus lifetime quarantine, bounded
health reads under large history, retention and one/all deletion, and
`PRAGMA integrity_check`.

### Setup and adapters

For each harness, run setup dry-run, setup, repeated setup, interrupted setup, and
uninstall. Separately run install-never-activated, restore with missing/empty state,
repeated restore, corrupt/partial state, concurrent user edits, symlink/non-regular
file refusal, and independent subsystem-cleanup failure. No managed state must be a
successful no-op. Compare before/after files byte-for-byte outside the managed block.
Prove one canonical settings contract or patch semantics: an omitted UI field cannot
reset an unrelated guard, and unknown fields are rejected where exactness is required.
Prove durable recovery intent, backup creation, permissions, origin reporting, and
exact native resume ID.
Version fixtures by the harness version that produced them. Unknown versions must
show reduced coverage instead of silently parsing as a known format.

### Product journeys

Capture onboarding, no-harness, empty-history, active ingest, degraded coverage,
quarantine, lock contention, dashboard, filtered search, session detail, resume,
retention preview, one-session deletion, confirmed delete-all, installed hooks/skills/
plugins/MCP inventory, MCP enable/disable, plugin verify,
secret-guard findings, AI-analysis preview/confirm/cancel/error/success, doctor,
and bundle. Cover keyboard-only navigation, focus restoration, reduced motion,
screen-reader names, loading, empty, partial, and error states.

Use representative desktop viewports at 1280×800 and 1536×960, plus a constrained
900×700 window. This is a desktop application; do not invent a phone layout, but
do prove the minimum supported window does not overflow or hide actions.

### Security and privacy

Pass only synthetic secrets through headers, URLs, nested JSON, tool output,
multiline text, and split stream chunks. Scan spool, database projections, logs,
crash files, screenshots, proxy traces, and diagnostic bundles for those sentinels.
Also include benign high-entropy values to catch destructive false positives.
Proxy activation must require consent, remain visibly active, fail open for
instrumentation, and never fabricate context breakdown when unavailable.

### Reconciliation

For fixture sessions, independently calculate tokens, costs, agents, tools, skills,
quota windows, and coverage. Dashboard and CLI totals must match. Conflicting
sources use declared precedence and never double count. Costs cite the pricing
snapshot and effective interval; unknown price remains unknown.

## Evidence

Save exact commands and exit codes as text or JSON logs. Browser/native journeys
record step-by-step actions, expected/actual outcomes, viewport, revision, and
screenshots. Installed-harness probes record the executable version and which
operations were genuinely exercised. Package checks record artifact hashes and
install location.

Every finding has a stable key, owner (`core`, `harness-claude`, `harness-codex`,
`harness-opencode`, `extensions`, `cli-ops`, `desktop`, or `integration`), severity,
reproduction, expected/actual result, and evidence path. Retest the exact journey
after repair and then rerun any affected aggregate checks.

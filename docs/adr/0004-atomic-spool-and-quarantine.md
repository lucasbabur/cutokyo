# ADR 0004: One event per atomic spool rename

- Status: Accepted
- Date: 2026-09-19

## Context

Cross-platform concurrent append calls do not provide a sufficient integrity contract
for shared JSONL logs. Hooks must capture while the main application is closed, and a
truncated record must not wedge the next drain.

## Decision

A hook creates a unique temporary file with create-new semantics, writes exactly one
valid JSON value plus one newline, flushes and syncs it, and atomically renames it into
`spool/`. Hooks never open SQLite. The owning core processes finalized names only.

Malformed or truncated files move durably to quarantine and update persisted health
before the ingest cursor advances. Drain then continues. Duplicate observation IDs
are harmless replays. Spool readers remain for every released integer format.
Retention is bounded by 30 days and 500 MB; reaching either cap visibly refuses new
capture instead of silently deleting history.

## Consequences

The spool contains more small files, trading directory overhead for simple integrity,
replay, and crash recovery. Cleanup may remove a source file only after complete,
durable ingestion. Current quarantine and lifetime quarantine are separate health
dimensions.

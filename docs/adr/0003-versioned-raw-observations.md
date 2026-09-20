# ADR 0003: Immutable raw evidence and attributable projections

- Status: Accepted
- Date: 2026-09-19

## Context

Harness formats drift, duplicate events arrive, resume identities can disagree, and
multiple channels report overlapping usage. A normalized-only database would erase
the evidence needed to repair parsers or explain totals.

## Decision

Raw observations are immutable while retained. Prefer a native event ID; otherwise
canonicalize and fingerprint the raw payload. Sequence numbers and mutable session
identifiers are not sufficient identities. The database uniquely indexes
`observation_id` for idempotency.

Every projection records source channel, capture time, native event/session/resume
identity, parser and derivation version, confidence, coverage, and all raw observation
links. Projections are rebuilt when `derive_version` changes. Unknown remains unknown;
estimates, user declarations, conflicts, and unavailable coverage remain labelled.
Unknown input versions are preserved raw without applying an older parser.

## Consequences

Storage costs more than normalized-only rows, but reconciliation is auditable and
parser fixes are recoverable. Explicit history deletion must remove linked raw rows,
derived rows, FTS entries, and summaries together. Fixtures must carry synthetic
provenance and expected uncertainty, not just dashboard totals.

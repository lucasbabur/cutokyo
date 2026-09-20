# ADR 0005: Recovery intent precedes configuration mutation

- Status: Accepted
- Date: 2026-09-19

## Context

Setup touches files owned by other harnesses and often shared with user entries. A
crash, symlink, concurrent edit, or broad uninstall could lose unrelated
configuration. Different clients can also reset settings they did not display.

## Decision

Before its first external mutation, setup durably records recovery intent. It accepts
only regular non-symlink targets, captures a snapshot, creates one backup, and rechecks
the snapshot immediately before an atomic write. Host structure, unmanaged bytes, and
permissions are preserved. Strict JSON is edited structurally; ownership markers are
used only where the syntax permits them.

Uninstall removes uniquely identifiable Cutokyo-owned nodes. No managed state,
empty state, and repeated cleanup are successful no-ops. Partial/corrupt state,
concurrent user edits, and independent subsystem failures are explicit outcomes;
one failed cleanup prevents another safe independent cleanup.

The canonical JSON Schema defines settings. Commands accept omission-preserving
patches and reject unknown public fields. Secrets stay in the OS keychain.

## Consequences

Mutation requires more bookkeeping and comparison, but recovery is deterministic and
neighboring configuration survives. Every setup adapter needs disposable-home tests
for apply, repeat, interruption, no-state, corruption, concurrent edit, and uninstall.

# Architecture

## Contents

- [System shape](#system-shape)
- [Dependency boundary](#dependency-boundary)
- [Capture and ingestion](#capture-and-ingestion)
- [SQLite ownership](#sqlite-ownership)
- [Configuration and secrets](#configuration-and-secrets)
- [Versioned contracts](#versioned-contracts)
- [Operational surfaces](#operational-surfaces)
- [Distribution boundary](#distribution-boundary)

## System shape

Cutokyo is one local Rust application with two frontends. The CLI is the
operational and automation surface. The Tauri desktop is a native presentation
surface over the same application use cases. Neither frontend owns a parallel
persistence implementation.

```text
harness hooks/plugins ──atomic files──> spool/
                                          │
                                          v
CLI ───────────────┐             cutokyo-core::ingest
                   ├─> cutokyo-core::app ─────────────> SQLite + FTS5
Tauri desktop ─────┘             │                    single writer
                                 └─> adapters/ports

cutokyo-domain: values, rules, provenance, errors, and ports; no I/O
```

## Dependency boundary

`cutokyo-domain` is pure. It cannot depend on filesystems, network clients, SQL,
process execution, Tauri, or an async runtime. `cutokyo-core` contains sibling
modules for application composition, ingest, storage, and adapters. Only the
application module wires those siblings into use cases.

Frontends may depend on `cutokyo_core::app`. Architecture tests reject store
imports from the CLI and desktop. A hook receives a spool-oriented application
surface, never a database path. External plugins receive versioned JSON-line
messages and declared data, never a connection or store handle.

## Capture and ingestion

A capture producer writes one JSON line per observation:

1. derive or retain a stable observation identity;
2. create a new temporary file in the spool filesystem;
3. write one bounded JSON line;
4. flush and synchronize it;
5. atomically rename it into the ready directory.

The drainer holds the writer lock, validates each format, and inserts by unique
observation ID. Malformed input moves durably to quarantine before the cursor
advances. A failed observation does not stop later entries. Current quarantine
and lifetime quarantine remain independent health dimensions.

Spool retention is a safety backlog cap, not history retention. At the age or
byte cap, capture stops visibly rather than deleting unseen evidence.

## SQLite ownership

The database is local disk only. Every connection enables WAL, foreign keys,
`synchronous=NORMAL`, and a five-second busy timeout. Exactly one process owns
the writer lock. Other frontends read or connect through the owner; they do not
steal a lock based on a guessed process state.

Backups use SQLite's online backup API. A restore must verify a digest before
replacement, retain the prior copy until integrity succeeds, and preserve a
recoverable state on interruption. History deletion removes linked raw,
derived, FTS, and summary rows transactionally but does not promise physical
SSD, WAL, or pre-existing backup erasure.

Process liveness and product readiness are distinct. A running binary may be
unready because configuration, integrity, schema, spool, lock, or plugin health
failed. A bounded persisted health projection gives the CLI, desktop, and
`doctor` the same answer without scanning ordinary full history.

## Configuration and secrets

Resolution order is deterministic:

1. compiled defaults;
2. one versioned user TOML file;
3. `CUTOKYO_*` environment variables;
4. command-line overrides.

`config list --show-origin` returns every candidate and marks the selected one.
A patch write reads persisted state rather than copying transient environment or
CLI values. Publication uses a same-directory atomic replacement, refuses
symlinks and non-regular files, rechecks the original bytes before replacement,
preserves existing permissions, and creates private files as `0600` where the
platform supports Unix modes.

Secrets are deliberately outside TOML. The default backend is the OS credential
store. An owner-only fallback is used only after explicit selection; it stores
hashed key filenames beneath a private directory and verifies permissions before
load.

## Versioned contracts

The surfaces evolve independently:

| Surface             | Version policy                                        |
| ------------------- | ----------------------------------------------------- |
| application         | semantic version from the release/tag                 |
| structured CLI      | semantic schema version; one-minor deprecation policy |
| database            | forward-only integer; newer schema refused            |
| derived projections | integer; rebuild instead of migration                 |
| spool               | integer; all released readers retained                |
| plugin protocol     | major handshake; unknown majors rejected              |
| JSON Schemas        | draft 2020-12 with stable `$id` and good/bad fixtures |

Unknown evidence remains unknown. It is never normalized to zero, false, or an
invented estimate. Every projection retains source tier, capture time, native
identity, parser version, confidence or coverage, and raw linkage.

## Operational surfaces

The CLI emits readable terminal output or a stable JSON success/error envelope.
Exit classes make shell automation distinguish usage, invalid contract,
unavailable capability, unhealthy state, and internal failure.

`tracing` writes bounded rotating JSONL logs. Ingest operations carry spans, but
raw prompts, paths, credentials, and observations are not log fields. Panics
write a bounded metadata-only record. The next launch advertises it; bundle
inclusion and clearing require explicit flags.

`doctor` evaluates configuration, permissions, harness availability, bundled
SQLite/FTS5 and pragmas, integrity, schema/derive versions, spool/quarantine,
persisted health, writer ownership, plugins, and safe aggregate counts. `bundle`
uses an allowlist and previews the archive before writing it.

## Distribution boundary

cargo-dist builds `cutokyo`, the only CLI implementation. Its generated npm
package downloads that platform-native executable and exposes it through
`node_modules/.bin/cutokyo`; it contains no TypeScript product implementation.
Artifact smoke tests unpack or install the package and resolve the executable
inside the artifact tree rather than falling back to `cargo run` or `target/`.

Every build/package invocation runs inside `tools/scripts/source-snapshot.py`,
which compares SHA-256 digests of all Git-tracked files before and after the
command and exits 86 on a rewrite. Releases add SHA-256 checksums, CycloneDX
SBOMs, GitHub build provenance, platform code signing, and Tauri updater
signatures. Tests and manual dry-runs never publish.

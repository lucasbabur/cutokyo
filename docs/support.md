# Support and troubleshooting

## Contents

- [Supported scope](#supported-scope)
- [First response](#first-response)
- [Exit codes](#exit-codes)
- [Common diagnoses](#common-diagnoses)
- [Safe issue material](#safe-issue-material)
- [Security reports](#security-reports)

## Supported scope

Cutokyo is a pre-1.0 community project. Support covers the public local-only
product, schemas, installers, and documented platform jobs. It does not provide
a hosted account, model-provider support, enterprise tenancy, remote database
administration, or recovery guarantees for unverified manual database edits.

A Linux check does not prove macOS or Windows packaging. A running process does
not prove product readiness. Harness drift may make a capture tier unavailable;
that state must be visible rather than guessed into success.

## First response

Start with commands that do not export private content:

```bash
cutokyo --json version
cutokyo config list --show-origin --json
cutokyo spool status --json
cutokyo doctor --json
```

Preserve the command, exit code, app/contract versions, operating system and
architecture, installation method, harness names and versions, and whether the
problem reproduces with a disposable config/data directory. Do not paste session
content while describing a search or capture failure.

Before any mutation, create and verify an online backup if the database opens.
Do not copy a live SQLite `.db`, delete WAL files, steal a writer lock, or edit
the schema manually.

## Exit codes

| Exit | Class          | Meaning                                                       |
| ---: | -------------- | ------------------------------------------------------------- |
|    0 | success        | command completed; inspect readiness fields where applicable  |
|   64 | usage          | unsupported command shape or invalid user argument            |
|   65 | contract       | malformed/unsupported structured input or protocol major      |
|   69 | unavailable    | capability, target, writer ownership, or state is unavailable |
|   70 | internal       | unexpected implementation or operating-system failure         |
|   78 | unhealthy      | doctor or bounded capacity reports product degradation        |
|   86 | source rewrite | build/package command modified a Git-tracked source file      |

Structured failures use the same safe error classes. The message avoids raw
secrets and unbounded external output.

## Common diagnoses

### Process is alive, doctor is nonzero

This is expected when product readiness failed. Read individual doctor check IDs
for config, permissions, harnesses, SQLite/FTS5, integrity, schema/derive,
spool/quarantine, persisted health, writer ownership, plugins, and aggregate row
counts. Fix one dimension at a time; one successful write does not clear another
failure.

### Writer is already owned

Do not remove the lock because a PID appears stale. Find the owning Cutokyo
process and shut it down normally. Another frontend may read or connect through
the owner but cannot steal write ownership.

### Quarantine is nonzero

Keep the quarantined file private. Doctor reports counts and safe identifiers,
not payloads. Correct the producer/protocol issue, drain later valid entries,
and acknowledge a quarantined entry only after review. Current and lifetime
quarantine are separate.

### Config write was cancelled

Cutokyo rechecks target bytes before atomic publication. A concurrent user edit
wins and produces a cancellation instead of being overwritten. Review
`config list --show-origin`, then retry a narrow patch. Symlinks and non-regular
targets are refused intentionally.

### Keychain is unavailable

The default secret operation fails closed. Repair/unlock the OS credential store
or explicitly choose the documented owner-only fallback after understanding that
it is a local permission boundary, not encryption against the active user.

### Bundle or log creation failed

Instrumentation degrades open so harness operation can continue. Check owner
permissions and disk capacity. Never work around the failure by attaching raw
logs, config directories, the database, or spool payloads.

### Old binary refuses the database

The database schema is newer than the binary. Reinstall the matching/newer
verified binary. Do not downgrade-open or edit the schema integer. If rollback is
required, use a verified backup created before migration.

## Safe issue material

A diagnostic bundle is optional and must be reviewed. Preview it first:

```bash
cutokyo bundle --json
cutokyo bundle --output ./cutokyo-diagnostics.tar.gz --json
```

Only add `--include-crash` after reviewing why the bounded crash record is
relevant. Extract the archive privately and inspect every entry. Public reports
may include versions, safe doctor output, coverage, aggregate counts, redacted
logs, exact exits, and artifact hashes.

Never attach prompts, transcripts, observations, database/backup bytes, spool or
quarantine payloads, raw logs, provider headers, credentials, keychain/fallback
files, unredacted configuration, or full local project paths.

## Security reports

Do not open a public issue for a suspected secret leak, signature bypass,
malicious update, arbitrary code execution, unsafe database replacement, or
privacy-boundary failure. Use the repository's private security-reporting
channel. Include minimal reproduction steps, affected version/hash, platform,
and safe impact description. Retain evidence privately and rotate exposed
credentials immediately; never include a real secret in the report.

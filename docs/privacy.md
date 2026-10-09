# Privacy, storage, and diagnostic disclosure

## Contents

- [Defaults](#defaults)
- [What is stored](#what-is-stored)
- [Secrets](#secrets)
- [Logs and crashes](#logs-and-crashes)
- [Diagnostic bundles](#diagnostic-bundles)
- [Provider egress](#provider-egress)
- [Deletion limits](#deletion-limits)
- [Safe sharing checklist](#safe-sharing-checklist)

## Defaults

Cutokyo defaults to local-only operation with no telemetry, no proxy capture,
no automatic MCP context injection, and no provider-bound analysis. Data and
configuration use platform-native user directories. Cutokyo is not a cloud
account and does not require a hosted service to search local history.

Native capture is preferred. A proxy is an explicit fallback for facts that a
native source cannot establish. Proxy capture requires its own content preview
and consent. Controls live in Settings.
This desktop build has no provider-proxy listener and reports that limitation
instead of claiming active capture.

Onboarding permits browsing without installing capture. Selecting a harness is
not consent to silently modify its configuration. Preview shows the native targets;
confirmation requires the plaintext-storage acknowledgement. Per-harness verification
checks configuration, not live events or complete transcript coverage. Neither
installation nor browsing enables proxy capture, telemetry, or provider analysis.
Installed hooks and the OpenCode plugin delegate to the selected local CLI receiver.
The receiver publishes private atomic spool files, never opens SQLite, and contains
capture failures so the native harness can continue.

## What is stored

Raw observations are immutable local evidence. They can include session content
provided by an enabled harness channel. Derived sessions, messages, search rows,
and summaries retain links to their evidence and record parser/source coverage.
A channel that is disabled, unavailable, or uninspectable is reported as unknown
coverage—not as an invented measurement.

Cutokyo `0.x` uses owner-only files where the operating system supports Unix
permissions, but the SQLite database is **not encrypted by Cutokyo**. Anyone who
can read the database file may be able to recover stored content. Use full-disk
encryption, a locked user account, and appropriate device backups.

## Secrets

Public settings may be written to the versioned TOML file. Keys whose names look
like tokens, passwords, credentials, API keys, or secrets are rejected by the
public config command.

The removed `outgoing_guard_enabled` field is not accepted in settings patches,
TOML, or CLI flags. Existing configuration containing it is rejected explicitly;
remove that obsolete field before loading it. No hidden enforcement switch remains.

Secret APIs first use the operating system credential store. If that service is
unavailable, Cutokyo fails closed unless the user explicitly selected the
documented owner-only fallback. Fallback values live outside TOML beneath a
private directory, use hashed filenames, and require `0600` files and a `0700`
directory on Unix. Neither backend makes a compromised user session safe.

## Logs and crashes

Structured logs are rotating JSONL files with a fixed byte cap and generation
count. Logging receives sanitized categories, identifiers, status, error codes,
and aggregate counts—not raw prompts, transcripts, observations, credentials,
or full project paths. Instrumentation failure does not block harness operation.

A panic record is bounded and metadata-only. It contains schema/app version,
process ID, a bounded `main`/`named`/`unnamed` thread classification, source
filename basename and line, and the `panic` category. Arbitrary thread names and
the panic payload are intentionally omitted. A later launch
advertises a pending record; Cutokyo does not upload it. Inclusion in a bundle
requires `bundle --include-crash`, and clearing it requires a successful bundle
plus `--clear-crash`.

## Diagnostic bundles

`cutokyo bundle` first previews a manifest. The archive allowlist is:

- bundle manifest and application/contract versions;
- safe effective configuration with secret-like values omitted;
- doctor output and baseline redaction receipt;
- aggregate database row counts;
- conservatively projected, redacted log records;
- the bounded crash record only after explicit opt-in.

The archive excludes prompts, transcripts, raw observations, raw secrets, full
project paths, database bytes, backup bytes, and spool/quarantine payloads. Each
entry is bounded, archive metadata is deterministic, and the result is private
where supported. A SHA-256 receipt detects later changes; it is not encryption.

A bundle is designed to be safer to share, not magically anonymous. Preview it,
extract it in an isolated directory, and inspect every file before sending it.

## Provider egress

AI analysis (CLI only; the desktop app has no analysis feature) and any provider-bound feature must show, before transmission:

- exact source session IDs;
- categories and extent of content leaving the device;
- provider and model;
- prompt contract version;
- baseline redaction performed;
- unavailable or uninspectable channels.

Confirmation is explicit. Cancel sends nothing. Retries use an idempotency key,
and a stored summary records provider, model, prompt version, source IDs, and
coverage. Baseline redaction of retained diagnostics and consented analysis is
not an outgoing traffic filter. Proxy requests pass unchanged to the harness's
configured provider/model; credentials and raw payloads are not retained in
proxy traces. Native capture coverage does not imply universal secret scanning.

## Deletion limits

One-session deletion, retention application, and delete-all remove linked raw,
derived, FTS, and summary rows from the active Cutokyo database. Destructive
commands preview their scope and require exact confirmation.

This is logical deletion, not certified secure erasure. It cannot alter existing
backups, filesystem snapshots, SSD remapping, previously exported bundles, or
all historical WAL pages. Checkpointing or vacuuming may reduce ordinary local
residue but does not justify a physical-erasure promise.

## Safe sharing checklist

1. Run `cutokyo doctor --json` locally and understand failed checks.
2. Run `cutokyo bundle` without `--output` to inspect the preview.
3. Include a crash record only when it is relevant and reviewed.
4. Generate the archive into a private directory.
5. Verify the SHA-256 receipt after transfer.
6. Extract and inspect every member before sharing.
7. Never attach the database, spool directory, raw logs, config secret fallback,
   or harness transcripts to a public issue.

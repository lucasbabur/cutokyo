# Local backups and restore

## Contents

- Create a backup in the desktop
- Restore history without handling digests
- Use the CLI
- Understand what a backup contains
- Recovery and verification tests

## Create a backup in the desktop

Open **Data controls**, then **Create backup**. Leave the location blank to use a private, timestamped directory under your Cutokyo data directory's `backups` folder. The optional location field accepts a new directory path. Cutokyo refuses an existing destination instead of replacing it.

The completion dialog shows the actual directory, database size, creation date, and session count. Treat the whole directory as one backup. It contains `history.db` and `history.db.manifest.json`; keep them together when moving a copy to another disk.

The available-backups list finds backups in the default directory and remembers custom locations created by this installation. A missing or incomplete directory does not appear as a usable backup. Use **Restore a backup from another location** to review a copy that you moved yourself.

## Restore history without handling digests

Choose **Restore** beside a backup. Cutokyo verifies its digest, checks SQLite integrity, and shows the selected contents and current session count. Confirm once with **Restore selected backup**. Cancelling the review changes no history.

The confirmation belongs to that exact backup and current history. If either changes before the restore starts, Cutokyo refuses the old preview. Review it again rather than entering a hash yourself.

During creation or restore, the dialog cannot close and its action cannot run twice. An error stays inside the dialog. A successful restore reloads the history view and reports the retained recovery copy of the previous database. That recovery directory appears in the available-backups list and can itself be restored.

Cutokyo prepares and validates the selected snapshot privately, including supported schema migration and derived-history rebuild. It then uses SQLite's online backup API to restore into the existing live database. It does not rename the live file or unlink its WAL. An open reader keeps its coherent transaction snapshot, and its next transaction sees the restored history. If applying or verifying the restore fails, Cutokyo automatically copies the retained prior snapshot back into the same live database. Recovery errors report where that prior snapshot remains.

## Use the CLI

Create a default backup:

```sh
cutokyo backup
```

Choose a new backup directory:

```sh
cutokyo backup /path/to/new-history-backup
```

Preview a restore without changing history:

```sh
cutokyo restore /path/to/history-backup
```

Restore with explicit confirmation:

```sh
cutokyo restore /path/to/history-backup --confirm
```

Add `--json` for machine-readable receipts. Creation returns `path`, `createdAt`, `byteLength`, `sessionCount`, and `scope`. Restore returns `recoveryPath`, `restoredSessionCount`, and the measured `integrityResult`. No digest argument or separate verification command is required.

Only the process owning Cutokyo's single-writer lock can create or restore history. A read-only second desktop window refuses these operations rather than taking the writer lock.

## Understand what a backup contains

This is a database backup. It includes stored local session history, immutable raw observations, search projections, summaries, and the database's operational records. It excludes settings files, harness configuration, pending spool entries, logs, and external plugin files. Restoring history does not restore those files or remove pending capture events.

Cutokyo uses SQLite's online backup API, not a file copy of the live WAL database. It hashes the closed backup and records its schema and integrity result. Backup reads use SQLite's immutable mode and refuse unexpected WAL, shared-memory, or journal sidecars. Restore verifies an exact staged copy again before changing history.

Cutokyo stages a complete database and manifest in a private sibling directory. Atomic no-replace publication refuses even a destination created while the backup is running. Failed publication cleans only private staging and preserves the unrelated destination. If a directory sync or optional-location registration fails after publication, the error reports the complete backup's retained path instead of deleting it.

On Unix, backup directories use owner-only `0700` permissions and database and manifest files use `0600`. Backups are not application-encrypted. Use full-disk encryption and control the lifetime of copies on other disks. Deleting history does not rewrite old backups, filesystem snapshots, WAL remnants, or SSD blocks, and Cutokyo does not promise physical secure erasure.

## Recovery and verification tests

The implementation and tests were written in this repository against its current contracts. No predecessor code or assets were used. The work reuses the pinned `rusqlite` online backup API, `tempfile`, and `url`. It uses maintained `rustix` no-replace directory publication on Linux and macOS. The third-party record documents its release and license. It adds no native dialog dependency; the default location works without a file picker.

Regression tests cover genuine SQLite restore and reopened search, backup tampering, unmanifested WAL refusal, immutable reads without sidecars, stale confirmation, retained prior history, injected post-apply and sync failures, failed publication cleanup, late destination collisions, owner-only permissions, unsafe writable ancestors, live-path aliases, missing current search indexes, older derivation counts, and overlapping live readers. Native service and CLI tests use isolated roots. React tests cover create/list/restore, cancellation, duplicate actions, disabled dismissal, modal-local errors, and history reload. Browser fixtures remain in-memory models and are not proof of native persistence.

The native package journey creates a backup, deletes history through the visible confirmation, restores through the visible dialog, checks the refreshed history list, opens the recovered session, rejects a tampered copy, and rediscovers the backup after an application restart. Core IPC assertions independently check the native session counts.

# Cutokyo

Cutokyo is a pre-1.0, local-only community observability and control surface for
Claude Code, Codex, and OpenCode. It preserves attributable session history,
searches it locally, resumes the exact native session, and reports capture and
storage health. The product is one native Rust application with CLI and Tauri
frontends. The npm package is only cargo-dist's generated installer for that
native executable.

## Contents

- [Product boundary](#product-boundary)
- [Clean install](#clean-install)
- [Desktop workflows](#desktop-workflows)
- [CLI tour](#cli-tour)
- [Architecture](#architecture)
- [Privacy and storage](#privacy-and-storage)
- [Development](#development)
- [Documentation](#documentation)
- [Contributing](#contributing)

## Product boundary

Cutokyo is a local session-history, provenance, exact-resume, configuration,
and diagnostics tool. Native hooks, documented local APIs, and OTel precede any
proxy fallback. Proxy capture and CLI-only provider-bound analysis are opt-in and must
preview egress before confirmation. The desktop app has no analysis feature.

Cutokyo is not a coding harness, model provider, cloud account, hosted memory
service, telemetry collector, tenant control plane, or invisible
man-in-the-middle. Every product feature in the specification is a community
feature under Apache-2.0; there is no proprietary-feature placeholder.

This is a `0.x` contract. App, database, derived projection, spool, plugin, and
structured CLI surfaces have explicit independent versions. New binaries
migrate supported old data forward; old binaries refuse a newer database.
There is no compatibility path from an unrelated predecessor product.

## Clean install

Prerequisites for source development are Rust 1.98.1, Node 22.22.3, pnpm
11.25.0, Python 3, and the platform's Tauri prerequisites when building the
desktop.

A released CLI may be installed with cargo-dist's generated shell, PowerShell,
or npm installer. All three install the same Rust binary. Verify the published
SHA-256 manifest before first launch.

```bash
# Preview only: creates no config or data directory.
cutokyo --json setup --dry-run

# Apply Cutokyo-owned setup state.
cutokyo setup

# Separate process liveness from product readiness.
cutokyo --json version
cutokyo --json doctor

# Repeatable removal preserves local history and user configuration.
cutokyo uninstall
```

Read the platform paths, package verification, no-state uninstall, and rollback
procedure in [`docs/clean-install.md`](docs/clean-install.md).

## Desktop workflows

Onboarding starts with no harness selected. **Browse without installing** saves
that choice without changing native capture configuration. To enable capture,
preview a harness, review its exact targets, acknowledge plaintext local storage,
and confirm installation. Selecting a checkbox alone installs nothing. Finish
rechecks each selected integration before saving it. Configuration verified does
not mean live capture verified; restart the harness and wait for native evidence.
Recover follows recorded installation or cleanup intent. Removal deletes only
unchanged Cutokyo-owned entries and retains modified user entries with an issue.

Automatic capture setup is verified for Claude Code `2.1.278`, Codex `0.153.4`,
and OpenCode `1.18.28`. Other versions, missing binaries, unsafe paths, or
uninspectable configuration make automatic setup unavailable. Browse still works.
Setup, recovery, and removal currently require the tested harness binary and the
Cutokyo receiver. A removed or upgraded harness can therefore prevent recovery;
restore the supported binary before retrying. POSIX command hooks are unavailable
on Windows in this build.

**Agent tools** is the place to manage installed MCP servers, skills, hooks,
plugins, and instruction files. Search by name or source, filter by harness or
kind, and choose **Manage** to read the actual source. Editable installations
support saving changes and removal with a retained recovery copy. Cutokyo-owned
capture and search entries explain why they cannot be edited here.

A skill or supported MCP server can be installed into another harness as an
independent copy. The preview names the destination and explains unavailable
formats or credential substitutions. Existing installations are never
overwritten. Shared sources show every affected harness; removing one shared
source affects all of them. Hook and plugin formats are harness-specific and
are not advertised as portable.

**Sessions** supports literal terms and exact phrases, relevance or newest
ordering, match excerpts, filters, and paginated results. Returning from session
detail keeps the search and page. **Today** means the machine's local calendar
day. Resume preserves the exact native identity and recorded project directory,
then opens a visible interactive terminal. Missing terminal, harness, or display
access fails with a retryable explanation. Success means the native process
acknowledged a real TTY and survived startup. It does not prove session activation;
check the terminal for native login or session errors. Codex hook session IDs
alone are not verified App Server thread IDs and cannot authorize resume.

**Data controls** separates saving a retention policy from deleting history.
Create a backup with one action at the default private location, then review
and confirm a verified restore without copying a digest. Read the scope and
recovery details in [Local backups and restore](docs/backup-and-restore.md).

**Health** offers scoped retries and a previewed local diagnostic report.
Unavailable updater or proxy services disclose the reason rather
than implying a working provider or listener. The guardrails feature is not
included; local redaction, explicit egress consent, and file/database integrity
checks remain.

## CLI tour

Every command has readable terminal output and a stable `--json` envelope. JSON
errors carry a safe error code and commands use distinct exits for usage (64),
invalid contracts (65), unavailable capabilities (69), internal failures (70),
and unhealthy state (78).

```bash
cutokyo sessions search --query "migration" --project my-project --json
cutokyo sessions show SESSION_ID
cutokyo sessions resume SESSION_ID            # exact command preview
cutokyo sessions resume SESSION_ID --execute  # acknowledged visible-terminal startup

cutokyo capture-setup --harness claude_code --dry-run
cutokyo capture-setup --harness claude_code
cutokyo capture-setup --harness claude_code --recover
cutokyo capture-setup --harness claude_code --uninstall

cutokyo retention preview --days 30 --write-plan retention-plan.json
cutokyo retention apply --plan retention-plan.json --confirm-digest DIGEST
cutokyo sessions delete SESSION_ID --confirm-session SESSION_ID
cutokyo sessions delete-all --confirm 'DELETE ALL CUTOKYO HISTORY'

cutokyo config list --show-origin
cutokyo spool status
cutokyo drain
cutokyo plugin verify ./my-plugin
cutokyo mcp manifest --json
cutokyo doctor --json
cutokyo bundle --output cutokyo-diagnostics.tar.gz
cutokyo backup                              # private default backup directory
cutokyo backup ./cutokyo-history-backup       # optional new directory
cutokyo restore ./cutokyo-history-backup      # verified preview only
cutokyo restore ./cutokyo-history-backup --confirm

cutokyo inventory list --json
cutokyo inventory show ITEM_ID --json
cutokyo inventory edit ITEM_ID --revision REVISION --content-file edited-source.json
cutokyo inventory install ITEM_ID --revision REVISION --harness opencode
cutokyo inventory remove ITEM_ID --revision REVISION --confirm-item ITEM_ID
```

Destructive commands preview exact scope and require a command-specific
confirmation. Deletion does not claim physical erasure from SSD blocks, WAL
pages, or prior backups.

## Architecture

```text
cutokyo-cli             cutokyo-desktop
     └─────────────┬──────────────┘
                   v
            cutokyo-core::app
         ┌─────────┼──────────┐
      ingest      store     adapters
         └─────────┼──────────┘
                   v
            cutokyo-domain
      no fs / network / SQL / process
```

Hooks only publish one flushed spool file by atomic rename and never open
SQLite. A CLI or desktop core owns the single writer lock. Frontends invoke
application use cases rather than store internals. See
[`docs/architecture.md`](docs/architecture.md).

## Privacy and storage

Defaults are local-only, no telemetry, no proxy, and no provider egress.
Configuration and data use platform-native directories. Owner-only permissions
are enforced where supported; secret values use the OS keychain or a separately
approved owner-only fallback and never TOML.

Cutokyo `0.x` does **not** encrypt the SQLite database at the application layer.
Session content may be plaintext on local disk, so full-disk encryption is
recommended. Diagnostic bundles contain an allowlist of generated diagnostics,
not prompts, transcripts, observations, database bytes, spool payloads, raw
secrets, or full project paths. A bounded crash record is advertised on the next
launch and only enters a bundle after `--include-crash`.

Read [`docs/privacy.md`](docs/privacy.md) before capture, analysis, deletion, or
sharing a diagnostic bundle.

## Development

The verifier must pass its mutation selftest before its repository result is
trusted. Build and package commands are run through the tracked-source snapshot
guard.

```bash
python3 tools/fleet/cutokyo-gates.py selftest
python3 tools/fleet/cutokyo-gates.py all --root .
cargo fmt --check
python3 tools/scripts/source-snapshot.py -- cargo clippy --workspace --all-targets -- -D warnings
python3 tools/scripts/source-snapshot.py -- cargo test --workspace
cargo deny check licenses bans
pnpm --dir ui check
python3 tools/scripts/source-snapshot.py -- dist plan
```

The packaged native keyboard journey in `pnpm --dir ui test:e2e:tauri` currently
requires Linux, a running Hyprland session with XWayland, and an executable
`hyprctl` on the host runner's `PATH`. The runner resolves that system tool to its
absolute real path in `CUTOKYO_NATIVE_COMPOSITOR_CLI`; the test refuses a missing,
relative, non-regular, or non-executable CLI. Keep the host
`HYPRLAND_INSTANCE_SIGNATURE` and `XDG_RUNTIME_DIR` available to the runner. Only
its scoped compositor subprocess uses the host runtime directory; the application
keeps private HOME and XDG directories. Unsupported compositor/input prerequisites
fail the journey, without skipping it or falling back to global keys or window
focus commands. Run native GUI acceptance without another native runner in parallel.

CI lints once on Linux and builds/tests on Linux, macOS, and Windows. RustSec,
cargo-pup, cargo-machete, and Knip are visible non-gating drift reports. Release
tests produce local or dry-run artifacts only; publishing requires an authorized
tag and all signing credentials.

## Documentation

- [Product contract](docs/spec/product.md)
- [Architecture](docs/architecture.md)
- [Privacy and diagnostic bundles](docs/privacy.md)
- [Clean installation and uninstall](docs/clean-install.md)
- [Local backups and restore](docs/backup-and-restore.md)
- [Management workflow contract](docs/management-workflow-contract.md)
- [Plugin authoring](docs/plugin-authoring.md)
- [Release signing and notarization](docs/release/signing.md)
- [Release recovery](docs/release/recovery.md)
- [Support](docs/support.md)
- [Clean-room provenance](docs/provenance/clean-room-policy.md)
- [Independent decision ledger](docs/provenance/decision-ledger.md)

## Contributing

Contributions are accepted under Apache-2.0 and Developer Certificate of Origin
1.1. Every commit must be signed off with `git commit -s`. Read
[`CONTRIBUTING.md`](CONTRIBUTING.md), the clean-room policy, and the decision
ledger before changing product or test files.

Claude Code, Codex, OpenCode, OpenAI, Anthropic, Tauri, and SQLite are marks or
names of their respective owners. See [`TRADEMARK.md`](TRADEMARK.md).

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
- [CLI tour](#cli-tour)
- [Architecture](#architecture)
- [Privacy and storage](#privacy-and-storage)
- [Development](#development)
- [Documentation](#documentation)
- [Contributing](#contributing)

## Product boundary

Cutokyo is a local session-history, provenance, exact-resume, configuration,
and diagnostics tool. Native hooks, documented local APIs, and OTel precede any
proxy fallback. Proxy capture and provider-bound analysis are opt-in and must
preview egress before confirmation.

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

## CLI tour

Every command has readable terminal output and a stable `--json` envelope. JSON
errors carry a safe error code and commands use distinct exits for usage (64),
invalid contracts (65), unavailable capabilities (69), internal failures (70),
and unhealthy state (78).

```bash
cutokyo sessions search --query "migration" --project my-project --json
cutokyo sessions show SESSION_ID
cutokyo sessions resume SESSION_ID            # exact command preview
cutokyo sessions resume SESSION_ID --execute  # explicit process launch

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
cutokyo backup ./cutokyo-backup.db
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

CI lints once on Linux and builds/tests on Linux, macOS, and Windows. RustSec,
cargo-pup, cargo-machete, and Knip are visible non-gating drift reports. Release
tests produce local or dry-run artifacts only; publishing requires an authorized
tag and all signing credentials.

## Documentation

- [Product contract](docs/spec/product.md)
- [Architecture](docs/architecture.md)
- [Privacy and diagnostic bundles](docs/privacy.md)
- [Clean installation and uninstall](docs/clean-install.md)
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

# Cutokyo

Cutokyo is a pre-1.0, local-only community project for inspecting session history,
usage provenance, installed agent infrastructure, and health across Claude Code,
Codex, and OpenCode. It is being built as one native Rust CLI with an optional
Tauri desktop application.

## Contents

- [Status](#status)
- [Product boundary](#product-boundary)
- [Architecture](#architecture)
- [Privacy and storage](#privacy-and-storage)
- [Foundation checks](#foundation-checks)
- [Contributing](#contributing)

## Status

This revision is the executable contract foundation, not a finished release. It
contains compiling domain and application boundaries, versioned schemas, golden
fixtures, deterministic fake endpoints, architecture tests, and self-testing
static gates. Search, durable SQLite ingest, harness setup, plugin execution,
MCP routing, proxy capture, analysis, packaging, and the product UI are not yet
claimed complete. There is no stable API and no supported migration from any
predecessor product.

## Product boundary

Cutokyo is intended to preserve searchable local session history, exact native
resume targets, source coverage, usage uncertainty, and installed hooks, skills,
plugins, and MCP servers. Native capture always precedes proxy fallback. Proxy
capture and AI analysis require explicit consent.

Cutokyo is not a coding harness, model provider, cloud account, hosted memory
service, telemetry collector, tenant control plane, or invisible man-in-the-middle.
All features described in the product specification are community features under
Apache-2.0; none are placeholders for a proprietary edition.

Read the full contract in [`docs/spec/product.md`](docs/spec/product.md).

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

Hooks and plugins only spool one event per atomic file rename. A CLI or desktop
core owns the single SQLite writer lock. Frontends call application use cases and
do not write the database directly.

## Privacy and storage

The defaults are local-only and no telemetry. v0.x uses owner-only file
permissions and redacts secrets before persistence, but it does **not** provide
application-level database encryption. Stored transcripts may therefore be
plaintext on local disk; full-disk encryption is recommended. Diagnostic bundles
must omit prompts, transcripts, raw secrets, and full project paths.

Deletion removes Cutokyo's linked raw, derived, search, and summary rows. It does
not promise physical erasure from SSD blocks, WAL pages, or existing backups.

## Foundation checks

Prerequisites are Rust 1.98.1, Node 22 or newer, and pnpm 11.25.0.

```bash
python3 tools/fleet/cutokyo-gates.py selftest
python3 tools/fleet/cutokyo-gates.py all --root .
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir ui check
```

The static verifier must pass its mutation selftests before its repository result
is trusted.

## Contributing

Contributions are accepted under Apache-2.0 and the Developer Certificate of
Origin 1.1. Every commit must be signed off with `git commit -s`. Read
[`CONTRIBUTING.md`](CONTRIBUTING.md), the clean-room policy, and the decision
ledger before changing product or test files.

Claude Code, Codex, OpenCode, OpenAI, Anthropic, Tauri, and SQLite are marks or
names of their respective owners. See [`TRADEMARK.md`](TRADEMARK.md).

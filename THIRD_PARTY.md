# Third-party software and license record

Cutokyo source is Apache-2.0. Third-party dependencies remain under their own licenses.
This foundation ships no copied fonts, images, icons, or predecessor assets.

## Direct Rust dependencies

Metadata was verified from crates.io and each upstream repository on 2026-09-20.
Exact active versions are in `Cargo.lock`; optional researched dependencies are not
shipped until their feature implementation enables them.

| Package            | Foundation version | License           | Purpose/status                                                  |
| ------------------ | -----------------: | ----------------- | --------------------------------------------------------------- |
| serde              |            1.0.229 | MIT OR Apache-2.0 | Serialization used by domain and contracts                      |
| serde_json         |            1.0.151 | MIT OR Apache-2.0 | JSON values and boundary serialization                          |
| time               |             0.3.55 | MIT OR Apache-2.0 | RFC 3339 parsing without I/O                                    |
| jsonschema         |             0.56.0 | MIT               | Draft 2020-12 fixture validation; remote resolution disabled    |
| tempfile           |             3.27.0 | MIT OR Apache-2.0 | Isolated architecture mutation tests                            |
| uuid               |             1.26.1 | Apache-2.0 OR MIT | Collision-resistant temporary and spool filenames               |
| rusqlite           |             0.40.2 | MIT               | Bundled SQLite, FTS5, pragmas, and the online backup API        |
| rusqlite_migration |              2.6.0 | Apache-2.0        | Forward-only SQLite migration runner                            |
| fs4                |              1.1.0 | MIT OR Apache-2.0 | Cross-platform database-writer and spool-publication file locks |
| sha2               |             0.11.0 | MIT OR Apache-2.0 | SHA-256 observation identities and backup/bundle digests        |
| clap               |             4.5.48 | MIT OR Apache-2.0 | Native CLI parsing, help, and exact command contracts           |
| directories        |              6.0.0 | MIT OR Apache-2.0 | Platform-native per-user config and data paths                  |
| toml               |              0.9.7 | MIT OR Apache-2.0 | Strict versioned user configuration parsing                     |
| tracing            |             0.1.41 | MIT               | Structured instrumentation and ingest spans                     |
| tracing-subscriber |             0.3.20 | MIT               | Bounded JSONL subscriber output                                 |
| tar                |             0.4.44 | MIT OR Apache-2.0 | Deterministic diagnostic archive construction                   |
| flate2             |              1.1.2 | MIT OR Apache-2.0 | Gzip diagnostic archive compression                             |
| keyring            |              4.2.0 | MIT OR Apache-2.0 | Operating-system credential-store abstraction                   |
| tauri              |             2.11.5 | Apache-2.0 OR MIT | Optional desktop-runtime dependency; stable v2                  |
| tauri-build        |              2.6.3 | Apache-2.0 OR MIT | Official Tauri build-context generator                          |

## Researched feature dependencies

| Package | Planned version | License    | Decision                                                    |
| ------- | --------------: | ---------- | ----------------------------------------------------------- |
| rmcp    |           3.4.0 | Apache-2.0 | Use the official Rust MCP SDK instead of a custom MCP stack |

This row records a researched direction, not a claim that the corresponding feature
is implemented or distributed. See ADR 0008.

## JavaScript development dependencies

These packages are development tools, not a second product CLI and not bundled as
runtime application code.

| Package         | Version | License           |
| --------------- | ------: | ----------------- |
| TypeScript      |   7.0.2 | Apache-2.0        |
| Vitest          |   5.0.1 | MIT               |
| Oxlint          |  1.83.0 | MIT               |
| Prettier        |   3.9.8 | MIT               |
| @tauri-apps/cli |  2.11.4 | Apache-2.0 OR MIT |

## Build and policy tools

| Package         | Version used by foundation | License           | Purpose                                                       |
| --------------- | -------------------------: | ----------------- | ------------------------------------------------------------- |
| cargo-dist      |                     0.32.0 | MIT OR Apache-2.0 | Native archives and generated shell/PowerShell/npm installers |
| cargo-cyclonedx |                      0.5.9 | Apache-2.0        | Release CycloneDX SBOM generation                             |
| cargo-deny      |                     0.20.2 | MIT OR Apache-2.0 | Local license and dependency-policy acceptance run            |
| cargo-pup       |                      0.1.8 | MIT OR Apache-2.0 | Visible non-blocking architecture report                      |
| cargo-machete   |                      0.9.2 | MIT OR Apache-2.0 | Visible non-blocking unused-dependency report                 |
| Knip            |                     6.37.0 | ISC               | Visible non-blocking TypeScript surface report                |
| pnpm            |                    11.25.0 | MIT               | Locked TypeScript workspace package manager                   |

## Workflow actions and standards text

GitHub Actions are pinned to full commit SHAs in workflow files. DCO text is Developer
Certificate of Origin 1.1 from <https://developercertificate.org/>. The Code of Conduct
is adapted from Contributor Covenant 2.1 under its published attribution terms. The
Apache License text is the unmodified Apache-2.0 license.

## Transitive inventory rule

The allowed transitive license set includes MIT-0 and MPL-2.0. MIT-0 is permissive;
MPL-2.0 appears in Tauri build-time CSS parsing dependencies and remains subject to
its file-level terms. No MPL-covered source is copied into this repository.

`cargo deny check licenses bans` enforces the resolved Rust graph. The lockfile is the
machine-readable transitive inventory. Release builds must additionally produce a
CycloneDX SBOM and preserve package license files. Any new dependency or asset updates
this file with version, source, purpose, and verified license before merge; an unknown
or unlicensed component blocks distribution.

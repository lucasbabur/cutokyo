# Third-party software and license record

Cutokyo source is Apache-2.0. Third-party dependencies remain under their own licenses.
This foundation ships no copied fonts, images, icons, or predecessor assets.

## Direct Rust dependencies

Metadata was verified from crates.io and each upstream repository on 2026-09-20.
Exact active versions are in `Cargo.lock`; optional researched dependencies are not
shipped until their feature implementation enables them.

| Package | Foundation version | License | Purpose/status |
| --- | ---: | --- | --- |
| serde | 1.0.229 | MIT OR Apache-2.0 | Serialization used by domain and contracts |
| serde_json | 1.0.150 | MIT OR Apache-2.0 | JSON values and boundary serialization |
| time | 0.3.55 | MIT OR Apache-2.0 | RFC 3339 parsing without I/O |
| jsonschema | 0.56.0 | MIT | Draft 2020-12 fixture validation; remote resolution disabled |
| jsonc-parser | 0.33.2 | MIT | CST edits that preserve unmanaged JSON and JSONC bytes |
| keyhog-core | 0.5.86, git `058b28911fbe9db4b0957f13be003f5fb36eb1a4` | MIT OR Apache-2.0 | Secret finding types projected into Cutokyo's sanitized guard model |
| keyhog-scanner | 0.5.86, git `058b28911fbe9db4b0957f13be003f5fb36eb1a4` | MIT OR Apache-2.0 | Maintained secret scanner with decode, entropy, ML, and multiline features |
| keyring | 4.2.0 | MIT OR Apache-2.0 | OS credential-store access for analysis provider keys |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Isolated architecture mutation tests |
| uuid | 1.26.1 | Apache-2.0 OR MIT | Collision-resistant temporary and spool filenames |
| rusqlite | 0.40.2 | MIT | Bundled SQLite, FTS5, pragmas, and the online backup API |
| rusqlite_migration | 2.6.0 | Apache-2.0 | Forward-only SQLite migration runner |
| fs4 | 1.1.0 | MIT OR Apache-2.0 | Cross-platform database-writer and spool-publication file locks |
| rmcp | 3.4.0 | Apache-2.0 | Official Rust MCP SDK for Cutokyo's read server and upstream broker |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | Parsed HTTPS endpoints for MCP and analysis providers |
| schemars | 1.2.1 | MIT | MCP tool input schemas |
| sha2 | 0.11.0 | MIT OR Apache-2.0 | SHA-256 observation identities and backup digests |
| tokio | 1.53.1 | MIT | Bounded subprocess, timeout, cancellation, and MCP async runtime |
| tokio-util | 0.7.19 | MIT | Cancellation tokens for analysis requests |
| toml_edit | 0.25.15 | MIT OR Apache-2.0 | Structure-preserving managed TOML edits |
| zeroize | 1.8.2 | Apache-2.0 OR MIT | Provider credential memory cleanup |
| tauri | 2.11.5 | Apache-2.0 OR MIT | Optional desktop-runtime dependency; stable v2 |
| tauri-build | 2.6.3 | Apache-2.0 OR MIT | Official Tauri build-context generator |

## JavaScript development dependencies

These packages are development tools, not a second product CLI and not bundled as
runtime application code.

| Package | Version | License |
| --- | ---: | --- |
| TypeScript | 7.0.2 | Apache-2.0 |
| Vitest | 5.0.1 | MIT |
| Oxlint | 1.83.0 | MIT |
| Prettier | 3.9.8 | MIT |
| @tauri-apps/cli | 2.11.4 | Apache-2.0 OR MIT |

## Build and policy tools

| Package | Version used by foundation | License | Purpose |
| --- | ---: | --- | --- |
| cargo-dist | 0.32.0 | MIT OR Apache-2.0 | Pinned dry-run distribution planning |
| cargo-deny | 0.20.2 | MIT OR Apache-2.0 | Local license and dependency-policy acceptance run |
| pnpm | 11.25.0 | MIT | Locked TypeScript workspace package manager |

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

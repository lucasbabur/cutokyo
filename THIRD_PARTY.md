# Third-party software and license record

Cutokyo source is Apache-2.0. Third-party dependencies remain under their own licenses.
This foundation ships no copied fonts, images, icons, or predecessor assets.

## Direct Rust dependencies

Metadata was verified from crates.io and each upstream repository on 2026-09-20.
Exact active versions are in `Cargo.lock`.

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
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Atomic runtime writes and isolated architecture mutation tests |
| uuid | 1.26.1 | Apache-2.0 OR MIT | Collision-resistant temporary and spool filenames |
| rusqlite | 0.40.2 | MIT | Bundled SQLite, FTS5, pragmas, and the online backup API |
| rusqlite_migration | 2.6.0 | Apache-2.0 | Forward-only SQLite migration runner |
| fs4 | 1.1.0 | MIT OR Apache-2.0 | Cross-platform database-writer and spool-publication file locks |
| rmcp | 3.4.0 | Apache-2.0 | Official Rust MCP SDK for Cutokyo's read server and upstream broker |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | Parsed HTTPS endpoints for MCP and analysis providers |
| schemars | 1.2.1 | MIT | MCP tool input schemas |
| sha2 | 0.11.0 | MIT OR Apache-2.0 | SHA-256 observation identities and backup/bundle digests |
| toml_edit | 0.25.15 | MIT OR Apache-2.0 | Structure-preserving Codex and managed broker configuration edits |
| json5 | 1.3.1 | MIT | Structural read-only parsing of documented OpenCode JSON/JSONC configuration |
| ureq | 3.4.2 | MIT OR Apache-2.0 | Blocking, bounded requests to validated OpenCode loopback app-server origins |
| url | 2.5.8 | MIT OR Apache-2.0 | WHATWG parsing and encoded path/query construction for loopback URLs |
| tokio | 1.53.1 | MIT | Bounded subprocess, timeout, cancellation, and MCP async runtime |
| tokio-util | 0.7.19 | MIT | Cancellation tokens for analysis requests |
| zeroize | 1.8.2 | Apache-2.0 OR MIT | Provider credential memory cleanup |
| clap | 4.6.1 | MIT OR Apache-2.0 | Native CLI parsing, help, and exact command contracts; version shared with KeyHog's exact maintained dependency |
| directories | 6.0.0 | MIT OR Apache-2.0 | Platform-native per-user config and data paths |
| toml | 0.9.7 | MIT OR Apache-2.0 | Strict versioned user configuration parsing |
| tracing | 0.1.44 | MIT | Structured instrumentation and ingest spans; version shared with KeyHog's exact maintained dependency |
| tracing-subscriber | 0.3.20 | MIT | Bounded JSONL subscriber output |
| tar | 0.4.44 | MIT OR Apache-2.0 | Deterministic diagnostic archive construction |
| flate2 | 1.1.9 | MIT OR Apache-2.0 | Gzip diagnostic archive compression; version shared with KeyHog's exact maintained dependency |
| wait-timeout | 0.2.1 | MIT OR Apache-2.0 | Bounded release-artifact subprocess integration tests |
| base64 | 0.22.1 | MIT OR Apache-2.0 | Strict decoding of Tauri's outer updater-signature encoding |
| minisign-verify | 0.2.5 | MIT | Established Minisign parser and verifier used at release assembly |
| minisign | 0.9.1 | MIT | Test-only generation of valid and deliberately mismatched updater signatures |
| tauri | 2.11.5 | Apache-2.0 OR MIT | Optional desktop-runtime dependency; stable v2 |
| tauri-build | 2.6.3 | Apache-2.0 OR MIT | Official Tauri build-context generator |
| tauri-plugin-wdio-webdriver | 1.4.0 | MIT | Test-feature-only embedded native WebDriver server; release builds reject the feature |

## Release-test npm fixture

The cargo-dist generated npm installer has one runtime dependency. Its exact upstream
package is retained solely as an offline install-test input; it is not a Cutokyo CLI
implementation and is removed before release assembly.

| Package | Version | Artifact SHA-256 | License | Upstream |
| --- | ---: | --- | --- | --- |
| detect-libc | 2.1.2 | `270dec0fc06cff86481da8af2dd8f18dee6b602790b14ef0e1c2c18d7da39427` | Apache-2.0 | <https://www.npmjs.com/package/detect-libc/v/2.1.2> |

## OpenCode fixture reference artifacts

These official npm packages were downloaded only to inspect the exact public type
surface and create documented-synthetic fixtures. Their source is not copied or
bundled by Cutokyo.

| Package | Version | Artifact SHA-256 | License | Upstream |
| --- | ---: | --- | --- | --- |
| @opencode-ai/plugin | 1.18.28 | `37efb341b471660fec94f7455d5e7bdfc24f8b835b1c16c648cc7f3dbf669878` | MIT | <https://www.npmjs.com/package/@opencode-ai/plugin> |
| @opencode-ai/sdk | 1.18.28 | `2592fb2b2861271290033320c1b975a347f3ee2cc3fc44fb777a066f23741745` | MIT | <https://www.npmjs.com/package/@opencode-ai/sdk> |

Exact upstream tag `v1.18.28` resolved to commit
`22006d97652839999596a34a48ff6be7dbb40c6e`; the fixture manifest records which
shapes are package-derived versus locally observed.

## JavaScript development dependencies

These packages are development tools, not a second product CLI and not bundled as
runtime application code.

| Package | Version | License |
| --- | ---: | --- |
| @tauri-apps/api | 2.11.1 | Apache-2.0 OR MIT |
| React / React DOM | 19.3.0 | MIT |
| Lucide React | 1.47.0 | ISC |
| TypeScript | 7.0.2 | Apache-2.0 |
| Vite / React plugin | 8.3.0 / 6.1.1 | MIT |
| Vitest | 5.0.1 | MIT |
| Testing Library React / user-event | 16.3.3 / 14.6.7 | MIT |
| Playwright Test | 1.63.0 | Apache-2.0 |
| axe-core / axe Playwright adapter | 4.13.0 | MPL-2.0 |
| WebdriverIO CLI, runner, Mocha framework, and client | 9.31.9 | MIT |
| @wdio/globals / @wdio/types | 9.31.3 / 9.30.1 | MIT |
| @wdio/tauri-service | 1.4.0 | MIT |
| jsdom | 30.1.0 | MIT |
| Knip | 6.37.0 | ISC |
| Oxlint | 1.83.0 | MIT |
| Prettier | 3.9.8 | MIT |
| tsx | 4.23.13 | MIT |
| @tauri-apps/cli | 2.11.4 | Apache-2.0 OR MIT |

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

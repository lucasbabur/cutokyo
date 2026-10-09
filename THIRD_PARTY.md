# Third-party software and license record

Cutokyo source is Apache-2.0. Third-party dependencies remain under their own licenses.
This foundation ships no copied fonts, images, or icons from third parties. The one predecessor asset is the owner-authored Cutokyo brand mark, reused at the owner's direction (see `docs/provenance/decision-ledger.md` DL-026).

## Direct Rust dependencies

Metadata was verified from crates.io and each upstream repository on 2026-09-20.
Exact active versions are in `Cargo.lock`.

| Package | Foundation version | License | Purpose/status |
| --- | ---: | --- | --- |
| serde | 1.0.229 | MIT OR Apache-2.0 | Serialization used by domain and contracts |
| serde_json | 1.0.150 | MIT OR Apache-2.0 | JSON values and boundary serialization |
| serde-saphyr | 1.3.0 | MIT OR Apache-2.0 | Bounded skill YAML-frontmatter validation; verified 2026-10-04, upstream release 2026-09-16, 8.86M crates.io downloads; maintained alternative to deprecated serde_yaml |
| time | 0.3.55 | MIT OR Apache-2.0 | RFC 3339 parsing without I/O |
| jsonschema | 0.56.0 | MIT | Draft 2020-12 fixture validation; remote resolution disabled |
| jsonc-parser | 0.33.2 | MIT | CST edits that preserve unmanaged JSON and JSONC bytes |
| keyhog-core | 0.5.86, git `058b28911fbe9db4b0957f13be003f5fb36eb1a4` | MIT OR Apache-2.0 | Scanner input chunks for bounded baseline redaction; raw matches remain internal, with no findings API |
| keyhog-scanner | 0.5.86, git `058b28911fbe9db4b0957f13be003f5fb36eb1a4` | MIT OR Apache-2.0 | Maintained secret scanner with decode, entropy, ML, and multiline features |
| keyring | 4.2.0 | MIT OR Apache-2.0 | OS credential-store access for analysis provider keys |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Atomic runtime writes and isolated architecture mutation tests |
| uuid | 1.26.1 | Apache-2.0 OR MIT | Collision-resistant temporary and spool filenames |
| rusqlite | 0.40.2 | MIT | Bundled SQLite, FTS5, pragmas, and the online backup API |
| rustix | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Safe no-replace directory publication on Linux/macOS; verified 2026-10-04, release 2026-09-16, 1.20B crates.io downloads; already used transitively by tempfile |
| rusqlite_migration | 2.6.0 | Apache-2.0 | Forward-only SQLite migration runner |
| unicode-normalization | 0.1.25 | MIT OR Apache-2.0 | Unicode NFC composition before literal search tokenization; verified 2026-10-04, upstream release 2025-10-30, maintained unicode-rs crate already used transitively |
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
| termlauncher | 0.4.0 | MIT | Visible terminal command execution with separate arguments and working directory; upstream release 2026-09-08, crates.io 667 total downloads and 300 recent downloads checked 2026-10-04, <https://github.com/atomicptr/termlauncher> |
| which | 8.0.6 | MIT | Established native executable resolution before setup and interactive resume |
| shlex | 2.0.1 | MIT OR Apache-2.0 | Checked POSIX argument quoting for native hook command configuration |
| toml | 0.9.7 | MIT OR Apache-2.0 | Strict versioned user configuration parsing |
| tracing | 0.1.44 | MIT | Structured instrumentation and ingest spans; version shared with KeyHog's exact maintained dependency |
| tracing-subscriber | 0.3.20 | MIT | Bounded JSONL subscriber output |
| tar | 0.4.46 | MIT OR Apache-2.0 | Deterministic diagnostic archive construction; updated 2026-10-04 to the maintained 2026-05-18 patch release fixing RUSTSEC-2026-0067 and RUSTSEC-2026-0068 |
| flate2 | 1.1.9 | MIT OR Apache-2.0 | Gzip diagnostic archive compression; version shared with KeyHog's exact maintained dependency |
| wait-timeout | 0.2.1 | MIT OR Apache-2.0 | Bounded release-artifact subprocess integration tests |
| base64 | 0.22.1 | MIT OR Apache-2.0 | Strict decoding of Tauri's outer updater-signature encoding |
| minisign-verify | 0.2.5 | MIT | Established Minisign parser and verifier used at release assembly |
| minisign | 0.9.1 | MIT | Test-only generation of valid and deliberately mismatched updater signatures |
| tauri | 2.11.5 | Apache-2.0 OR MIT | Optional desktop-runtime dependency; stable v2 |
| tauri-build | 2.6.3 | Apache-2.0 OR MIT | Official Tauri build-context generator |
| tauri-plugin-wdio | 1.4.0 | MIT OR Apache-2.0 | Test-feature-only WebdriverIO bridge and native window inspection commands |
| tauri-plugin-wdio-webdriver | 1.4.0 | MIT | Test-feature-only embedded native WebDriver server; release builds reject the feature |

The visible-terminal launcher was selected after comparing `termlauncher` 0.4.0,
`open` 5.4.4, and `portable-pty` 0.9.0. `open` opens URLs and paths rather than
arbitrary interactive native commands. `portable-pty` supplies a PTY but does not
open a visible terminal window. `termlauncher` accepts separate arguments and a
working directory, so Cutokyo uses it rather than maintaining terminal-specific
launch commands. Its adoption is modest, as recorded above. Cutokyo adds a bounded
TTY/startup acknowledgement because a successful launcher exit alone is not proof
that the native harness started.

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
| @wdio/tauri-service / @wdio/tauri-plugin | 1.4.0 | MIT |
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
| PyYAML          |                      6.0.3 | MIT               | Development-only parsed workflow contract tests, verified from PyPI metadata |
| actionlint      |                     1.7.12 | MIT               | GitHub expression and shell validation, upstream `LICENSE.txt` |

## Workflow actions and standards text

GitHub Actions are pinned to full commit SHAs in workflow files. DCO text is Developer
Certificate of Origin 1.1 from <https://developercertificate.org/>. The Code of Conduct
is adapted from Contributor Covenant 2.1 under its published attribution terms. The
Apache License text is the unmodified Apache-2.0 license.

## Dependency maintenance verified 2026-10-04

Compatible updates retain Tauri/runtime pins and baseline redaction. `yoke-derive`
0.8.4 replaces the yanked 0.8.3 release. `tauri-utils` 2.10.1 replaces 2.9.3 and
resolves `urlpattern` 0.6.0 with maintained ICU properties instead of the five
unmaintained UNIC packages. Both releases were published 2026-09-30; all locked
parents accept their declared versions. These are transitive updates, with the
exact resolved graph and licenses recorded in `Cargo.lock` and enforced by
`cargo deny check licenses bans`.

The update also resolves upstream HTML/CSS, compression, file-type detection,
and constructor dependencies. Native packaged tests must exercise that graph.
`cargo deny check advisories` still fails on four unmaintained transitive crates:
`bitmaps`, `im`, and `sized-chunks` through current KeyHog/Vyre, and
`proc-macro-error` through Tauri's GTK 0.18 dependency. Current stable upstream
releases offer no compatible fix. No advisory ignores, dependency forks, or
removal of baseline redaction were added.

## Harness marks (desktop UI)

The desktop UI draws each supported harness's mark inline (`ui/src/components/HarnessMark.tsx`, single-path
SVG, `currentColor`) so that harness names are recognisable in filters, chips and tables. The marks identify
third-party products; their use is nominative and does not imply affiliation or endorsement. The marks remain
trademarks of their owners, are not covered by Cutokyo's licence, and `TRADEMARK.md` applies.

| Mark | Source | Source licence | Trademark note |
| --- | --- | --- | --- |
| Claude Code | `claudecode.svg` in npm `simple-icons` 16.34.0 (<https://github.com/simple-icons/simple-icons>) | CC0-1.0 (artwork); | Claude Code is a trademark of Anthropic, PBC |
| OpenCode | `opencode.svg` in npm `simple-icons` 16.34.0; same geometry as the mark published in the MIT-licensed <https://github.com/anomalyco/opencode> repository | CC0-1.0 (artwork); | OpenCode is a project of its maintainers; mark used to identify the harness only |
| Codex | `codex.svg` in npm `@lobehub/icons-static-svg` 1.95.1 (<https://github.com/lobehub/lobe-icons>) | MIT (package); | Codex and OpenAI are trademarks of OpenAI. `simple-icons` no longer ships an OpenAI mark, so this package is the maintained source |

The packages were fetched with `npm pack` only to copy the path data; neither is a runtime dependency.

## Transitive inventory rule

The allowed transitive license set includes MIT-0 and MPL-2.0. MIT-0 is permissive;
MPL-2.0 appears in Tauri build-time CSS parsing dependencies and remains subject to
its file-level terms. No MPL-covered source is copied into this repository.

`cargo deny check licenses bans` enforces the resolved Rust graph. The lockfile is the
machine-readable transitive inventory. Release builds must additionally produce a
CycloneDX SBOM and preserve package license files. Any new dependency or asset updates
this file with version, source, purpose, and verified license before merge; an unknown
or unlicensed component blocks distribution.

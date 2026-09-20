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
| serde_json | 1.0.151 | MIT OR Apache-2.0 | JSON values and boundary serialization |
| time | 0.3.55 | MIT OR Apache-2.0 | RFC 3339 parsing without I/O |
| jsonschema | 0.56.0 | MIT | Draft 2020-12 fixture validation; remote resolution disabled |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Isolated architecture mutation tests |
| uuid | 1.26.1 | Apache-2.0 OR MIT | Collision-resistant temporary and spool filenames |
| rusqlite | 0.40.2 | MIT | Bundled SQLite, FTS5, pragmas, and the online backup API |
| rusqlite_migration | 2.6.0 | Apache-2.0 | Forward-only SQLite migration runner |
| fs4 | 1.1.0 | MIT OR Apache-2.0 | Cross-platform database-writer and spool-publication file locks |
| sha2 | 0.11.0 | MIT OR Apache-2.0 | SHA-256 observation identities and backup digests |
| tauri | 2.11.5 | Apache-2.0 OR MIT | Optional desktop-runtime dependency; stable v2 |
| tauri-build | 2.6.3 | Apache-2.0 OR MIT | Official Tauri build-context generator |
| tauri-plugin-wdio-webdriver | 1.4.0 | MIT | Test-feature-only embedded native WebDriver server; release builds reject the feature |

## Researched feature dependencies

| Package | Planned version | License | Decision |
| --- | ---: | --- | --- |
| rmcp | 3.4.0 | Apache-2.0 | Use the official Rust MCP SDK instead of a custom MCP stack |

This row records a researched direction, not a claim that the corresponding feature
is implemented or distributed. See ADR 0008.

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

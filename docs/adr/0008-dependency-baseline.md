# ADR 0008: Maintained dependency baseline

- Status: Accepted for foundation; feature use remains reviewable
- Date researched: 2026-09-20

## Context

Cutokyo needs SQLite-native FTS and online backup, MCP interoperability, a desktop
shell, schema validation, and ordinary serialization. Custom substitutes would add
security and compatibility work without product value. Exact dependency versions make
the initial contract reproducible; Dependabot or deliberate updates can advance them.

## Research

Registry metadata and upstream release indexes were checked before pinning:

| Need | Selection | Evidence and decision |
| --- | --- | --- |
| SQLite | `rusqlite` 0.40.2 (MIT) | Current release from 2026-08-08 of the established, actively developed SQLite wrapper; exposes bundled SQLite, FTS5, backup, and low-level pragma APIs. |
| Migrations | `rusqlite_migration` 2.6.0 (Apache-2.0) | Released 2026-05-28, explicitly compatible with `rusqlite` 0.40, and used in Debian unstable; preferable to a custom migration runner. |
| File locking | `fs4` 1.1.0 (MIT OR Apache-2.0) | Current release from 2026-04-28, maintained across Unix and Windows, and independently used by Qdrant; preferable to platform-specific lock code. |
| Digests | RustCrypto `sha2` 0.11.0 (MIT OR Apache-2.0) | Current release from 2026-03-25 in the actively maintained RustCrypto hashes project; broadly adopted and preferable to a private SHA-256 implementation. |
| MCP | official `rmcp` 3.4.0 (Apache-2.0) | Official Model Context Protocol Rust SDK, current release, server/client and stdio/HTTP transport features; preferable to a private protocol stack. |
| Desktop | Tauri 2.11.5, `tauri-build` 2.6.3, and Tauri CLI 2.11.4 (Apache-2.0 OR MIT) | Stable v2 runtime and official companion tooling checked; CLI 2.11.4 is the established patch accepted by the workspace release-age policy. v3 prereleases were rejected for v0.x. |
| JSON Schema | `jsonschema` 0.56.0 (MIT) | Current maintained validator with draft 2020-12 support; default network/file resolution is disabled for offline fixture checks. |
| Data model | `serde` 1.0.229, `serde_json` 1.0.151, `time` 0.3.55, `uuid` 1.26.1 | Current established crates; domain IDs do not generate randomness and time parsing does no I/O. |
| Errors/tests | `thiserror` 2.0.20, `tempfile` 3.27.0 | Current established crates; `thiserror` is reserved for implementation errors and `tempfile` supports isolated tests. |
| TypeScript checks | TypeScript 7.0.2, Vitest 5.0.1, Oxlint 1.83.0, Prettier 3.9.8 | Current npm metadata and upstream repositories/licenses checked. |
| Distribution policy | `cargo-dist` 0.32.0 and `cargo-deny` 0.20.2 (MIT OR Apache-2.0) | Maintained tools with compatible Rust minima; the release workflow pins planning and local acceptance pins policy evaluation rather than downloading mutable latest versions. |

Primary upstreams:

- <https://github.com/rusqlite/rusqlite>
- <https://github.com/cljoly/rusqlite_migration>
- <https://github.com/al8n/fs4-rs>
- <https://github.com/RustCrypto/hashes>
- <https://github.com/modelcontextprotocol/rust-sdk>
- <https://v2.tauri.app/release/>
- <https://github.com/Stranger6667/jsonschema>
- <https://serde.rs/>
- <https://www.typescriptlang.org/>
- <https://vitest.dev/>
- <https://oxc.rs/docs/guide/usage/linter>
- <https://prettier.io/>
- <https://github.com/axodotdev/cargo-dist>
- <https://github.com/EmbarkStudios/cargo-deny>

Crates.io metadata also showed minimum Rust versions and licenses. GitHub release/tag
commits were resolved before pinning workflow actions. Popularity alone was not used;
fitness, official ownership, current maintenance, and license compatibility were
required.

## Decision

Pin the foundation toolchain and direct dependencies exactly. The durable store uses
`rusqlite`, `rusqlite_migration`, `fs4`, and `sha2`, with bundled SQLite selected to
prove FTS5 and online backup. Use official `rmcp` when MCP lands. Keep Tauri v2; do
not adopt a v3 prerelease. Do not add these I/O dependencies to `cutokyo-domain`.

Only dependencies already used by foundation code appear in active workspace
manifests; researched future choices are recorded rather than added decoratively.

## Consequences

The lockfile is reviewable and automated updates must rerun license, architecture,
schema, SQLite capability, MCP, and native desktop tests. Exact pins trade automatic
patch uptake for deliberate review in this security-sensitive pre-1.0 foundation.

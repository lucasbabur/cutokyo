# Agent guide

Cutokyo is a local-only observability and control surface for Claude Code, Codex
and OpenCode: one Rust application with a CLI and a Tauri desktop. Read
[`README.md`](README.md) for the product and [`docs/`](docs/) for depth.

## Before changing code

- Load the contract before touching domain, ingest, adapters, MCP, config or
  persistence: [`.claude/skills/cutokyo-contract/SKILL.md`](.claude/skills/cutokyo-contract/SKILL.md).
  Its rules (single writer, spool-only hooks, provenance, recovery before every
  native-config write, owned-entries-only removal) are not negotiable.
- Never read or copy the predecessor at `../cutokyo`; see
  [`docs/provenance/clean-room-policy.md`](docs/provenance/clean-room-policy.md).
- `cutokyo-domain` does no I/O. Frontends call `cutokyo-core::app` use cases and
  never touch the store directly.

## Layout

| Path                           | What lives there                                                                    |
| ------------------------------ | ----------------------------------------------------------------------------------- |
| `crates/cutokyo-domain`        | Pure values, rules, provenance, ports                                               |
| `crates/cutokyo-core`          | `app`, `ingest`, `store` (SQLite + FTS5), `adapters`, `mcp`, `inventory_management` |
| `crates/cutokyo-cli`           | The `cutokyo` command                                                               |
| `crates/cutokyo-desktop`       | Tauri shell; `service.rs` maps use cases to commands                                |
| `ui/`                          | React desktop UI; `src/fixtures` drive it in a browser without Tauri                |
| `tests/`                       | Cross-crate pipeline, upgrade and end-to-end tests                                  |
| `tools/fleet/cutokyo-gates.py` | Static contract gates                                                               |

## Checks

Run what your change touches; all of it before a commit.

```bash
python3 tools/fleet/cutokyo-gates.py selftest && python3 tools/fleet/cutokyo-gates.py all --root .
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm --dir ui check            # types, lint, format, unit tests, knip
pnpm --dir ui test:e2e:web     # Playwright on fixtures; needs port 4173 free
```

- Three `tests/e2e.rs` release tests (`release_artifact_manifest`,
  `npm_wrapper_install`, `release_contract_mutations`) need a committed tree and
  fail on uncommitted work. That is expected.
- `pnpm --dir ui test:e2e:tauri` builds the real app and drives real windows on
  the host's Hyprland session. Ask before running it on someone's desktop. It
  calls `cargo tauri`; if that is missing, put a `cargo-tauri` shim on `PATH`
  that runs `node_modules/.bin/tauri`.
- Commits need `git commit -s` and the trailer
  `Co-Authored-By: Claude Code <noreply@anthropic.com>`
  (`tools/scripts/check-commit-message.py`).

## UI work

- Browse any screen with fixtures:
  `VITE_CUTOKYO_BROWSER_FIXTURES=1 pnpm --dir ui dev --host 127.0.0.1 --port 4173`,
  then `/?jev_case=<case>#/<route>` (cases in `ui/src/fixtures/browserAdapter.ts`;
  `large-inventory` has ~230 tools, real-world scale).
- The desktop is WebKitGTK, not Chromium. Button padding, focus rings and some
  layout differ. Check pixel-level details in WebKitGTK (an offscreen
  `WebKit2.WebView` from Python `gi` is enough), not only in Playwright.
- Font sizes must use the `--font-size-*` tokens (`ui/src/designTokens.test.ts`).
- Keep UI text minimal: no eyebrows, disclaimers or explanatory badges.

## Data scale

Real profiles hold ~1 GB of history, ~500k evidence rows and sessions with
16,000+ observations. Never return unbounded evidence lists to the UI or MCP;
results carry the first `LISTED_EVIDENCE` IDs and an `observation_count`.

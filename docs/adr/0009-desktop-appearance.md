# ADR 0009: Desktop appearance and dark evidence surfaces

## Decision

Cutokyo offers **System**, **Light**, and **Dark** in desktop Settings. System follows the operating system as it changes; a selected override remains in the private desktop preference file, not the shared CLI configuration. The existing light workspace remains available. The dark workspace has its own graphite palette rather than an inverted light palette.

## Visual reference lock

Build target: the existing Cutokyo desktop UI, including its sidebar, page hierarchy, dense evidence tables, provenance disclosures, uncertainty language, and distinct health states. No imagery is needed: the product evidence, charts, and native icons are the content.

| Decision | Reference / constraint | Role retained |
| --- | --- | --- |
| Deep graphite canvas, slightly raised cards, hairline borders, minimal shadows | Refero: Linear Changelog style | Structural depth, not a decorative glow or accent wash |
| Legible compact charts and raw-value tables | Refero: Cursor usage dashboard; Cutokyo's existing evidence model | Harness hues encode identity; labels and table keep identity readable without color |
| Dense session and health rows with visible metadata | Refero: Parallel history table; existing Cutokyo layout | Information stays in place; color does not replace provenance or status text |
| Segmented Light / Dark / System preference, immediately previewed | Refero: Typefully appearance setting; Missive appearance flow | Appearance only; no implication that privacy or capture settings change |
| Honest empty states and recovery route | Refero: FlutterFlow empty state; Cutokyo's unknown-not-zero rule | Explain missing evidence rather than inventing zero values |
| Sparse cyan focus/link color; vermilion only for danger/live capture | Cutokyo product spec and existing interface | Never promote a semantic warning or series color into general decoration |

Token roles: `--surface-*` are workspace elevations; `--text-*` are readability levels; `--success-*`, `--warning-*`, and `--danger-*` remain semantic; `--series-*` are harness identity. Light tokens preserve the warm-paper workspace. Dark tokens use #111411 canvas and #1b1f1b cards, with text and status colors selected independently for contrast. Shadows recede in dark mode; selection, focus, native controls, and chart tracks adapt too.

## Verification criteria

Check 900×700, 1280×800, and 1536×960 in both themes, including settings, empty sessions, an evidence-rich dashboard, degraded health, and a confirmation modal. Check forced colors and focus. Validate the three categorical chart hues on the actual chart surfaces and retain labels plus the raw-value table. The appearance preference must survive restart; System must react to an OS preference change; a failed save must not falsely show a persisted selection.

## Verification record (2026-09-29)

- `pnpm --dir ui check`: 62 unit tests, 12 native-harness unit tests, settings contract, typecheck, lint, formatting, and unused-code checks passed. The production bundle excludes browser fixtures and the native test bridge.
- `pnpm --dir ui test:e2e:web`: 16 browser journeys passed on the final rerun. Its seven light fixture states and five dark fixture states were captured at all three viewport sizes in `evidence/final/screens/` and `evidence/theme/screens/`; the dark theme also has a forced-colors chart capture and a 900×700 conflict-recovery capture. One earlier run failed on the first onboarding page with a blank frame before the UI mounted, then passed on rerun; do not count that run as green.
- Dark dashboard axe checks at all three sizes and the 900×700 conflict screen reported zero violations. The forced-colors chart uses distinct mark textures and retains the labeled raw-value table.
- `cargo fmt --all --check`, warning-denying desktop Clippy, and `cargo test -p cutokyo-desktop --lib` passed (15 tests). Cross-window stale writes reject replacement of a saved theme; reloading settings permits a safe retry. UI regressions cover stale reads, newer external preferences, conflict reread, failed saves, and a delayed startup preference.
- `PATH=/tmp/cutokyo-tauri-cli/bin:$PATH pnpm --dir ui test:e2e:tauri` passed against the final Debian-packaged Tauri UI on X11/XWayland: exact resume, previewed deletion, 1280×800 native geometry, saved Dark, and restored Dark after a new app process. The final run's isolated evidence is under `target/native-evidence/attempt-kf26n1jh/`. A pure-Wayland native run was not verified.
- `cargo test --workspace --exclude cutokyo-desktop` is **not green** on this uncommitted branch: three release-artifact tests require committed source; the other 18 integration journeys in that suite passed. The release acceptance command was not weakened.

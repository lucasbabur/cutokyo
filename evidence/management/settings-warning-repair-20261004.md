# Desktop settings warning repair

## Contents

- Repair and warning behavior
- Installed profile and preserved history
- Executed checks
- Retained failures and limits

## Repair and warning behavior

The obsolete empty `mcp_enabled` desktop-state entry was removed explicitly after an owner-only recovery copy, an exclusive lock, and a source-byte recheck. The current persisted contract remains strict. No alias, compatibility reader, or automatic invalid-state rewrite was introduced.

All valid preferences were preserved: completed onboarding, Claude Code/Codex/OpenCode selections, Notify updates, disabled crash reports, and Dark appearance. Desktop state remains mode 0600.

The application banner owns startup warnings. Onboarding no longer repeats the same decode warning. Bootstrap, Settings, and Onboarding reread persisted state, clear a repaired decode notice, and retain unrelated core warnings. The banner has a Recheck local status action. Bootstrap and active-page refresh failures remain visible with Retry while cached data and pending setup choices remain mounted.

Independent source review found a retained-data refresh error that was initially hidden in Settings and Onboarding. Both pages now disclose it. Two whole-application regressions cover successful bootstrap accompanied by failed route refresh; the independent reviewer confirmed closure.

## Installed profile and preserved history

The ordinary production release-profile Debian application and matching CLI were installed at `~/.local/opt/cutokyo`. This is not the native-E2E-instrumented build. The complete previous installation and an integrity-checked SQLite online backup remain in a private installation recovery directory.

Live verification exposed a separate incompatibility: the existing `~/.local/share/cutokyo/cutokyo.db` reports schema 5, while this repository implements schemas through 3. A bounded inspection of this repository's source and available Git history found no schema-5 implementation. Its origin was not established. No predecessor implementation or schema was read.

The installed desktop and public CLI launchers now select a separate supported profile through the existing `CUTOKYO_DATA_DIR` and `CUTOKYO_CONFIG_FILE` overrides:

- Data: `~/.local/share/cutokyo-community`
- Config: `~/.config/cutokyo-community/config.toml`

The configuration and repaired desktop preferences were copied byte-for-byte into new owner-only directories. The original database was not deleted, replaced, downgraded, or imported. This profile starts new history. A second online backup after profile separation matched the pre-update online backup's SHA-256 exactly, and both integrity checks returned `ok`.

The exact verified application window was closed gracefully and reopened through the installed launcher. The new process executable hash matches the production installation record. Its environment selects the separate profile, its window is 1280×800, and its saved preferences remain unchanged. A private cropped header was read individually. The desktop decode and history-version startup warnings are absent. Distinct partial price/quota coverage information remains visible.

Installed app SHA-256: `b749908ec7ced700c8fd0264f3a8df66a834612c3aa33ac7f660fe20dc773064`.

Original-history online backup SHA-256: `8e97499ca8011c9f15937597d5b77391eefdb1d7a1bc4a56e566f470eb837b62`.

Operator harness configuration was not changed. Carrying over selected harness preferences does not establish that native capture has been installed or activated for the new profile.

Local installation/profile receipts are in `target/warning-fix-evidence/installation-update.json`, `community-profile.json`, `installed-community-launch-verification.json`, and `profile-readiness.json`. Operator configuration, history copies, and cropped operator screenshots remain private rather than entering repository evidence.

## Executed checks

| Command or check | Actual result |
| --- | --- |
| `pnpm --dir ui check` | Exit 0. 162 React tests, 25 Python runner tests, 34 Node keyboard-helper tests, TypeScript, lint, format, generated settings, and Knip |
| `cargo fmt --all -- --check && cargo test --locked -p cutokyo-desktop && cargo clippy --locked -p cutokyo-desktop --all-targets --all-features -- -D warnings` | Final exit 0. All 36 desktop tests passed; strict Clippy passed |
| `pnpm --dir ui test:e2e:web` | Exit 0. All 42 browser journeys passed |
| `pnpm --dir ui test:e2e:tauri` | Final exit 0 in 262.33 seconds. All 17 maintained native cases across six phases passed |
| `cargo build --locked --release -p cutokyo-cli --bin cutokyo && cargo tauri build --ci --features desktop-runtime --bundles deb` | Exit 0. Ordinary production build; no source or Git changes during execution |
| `python3 tools/fleet/cutokyo-gates.py selftest` and `all --root .` | Both passed |
| Explicit desktop-state repair helper | Four selftests passed |
| Installed CLI `--json doctor` | Exit 0, `ok: true` against the new profile |
| Read-only integrity checks | Original schema 5 and new schema 3 both returned `ok` |
| Installed launcher shell syntax and CLI version | Passed; CLI reports supported schema 3 |
| `git diff --check` | Passed |

The additive native state-recovery case begins with the exact obsolete entry in a fixture-free completed profile, proves the warning appears once and the invalid file stays unchanged, then performs explicit repair. Recheck restores completed-user navigation and Dark appearance without restarting or altering other preferences.

Native counts are save 9, reopen 3, browse 1, state recovery 1, capture setup 2, and capture history 1. All 16 native screenshots were read individually: the main agent inspected the new before/after pair, and an independent reviewer inspected the other 14 without finding a concrete visible warning-fix regression. Of 94 browser screenshots, 90 remained byte-identical to the previously inspected baseline and all four changed images were read individually.

Exact command records, original logs, source snapshots, and inspection receipts remain under `target/warning-fix-evidence`. The final native package and fixture evidence remain under `target/native-evidence/attempt-fodq11v5`.

## Retained failures and limits

The initial Rust command passed all 36 desktop tests but failed strict Clippy on a test-only function-pointer array type. A local type alias fixed it without suppression. The original failed output and successful exact retest are retained separately.

One native build was cancelled when independent review found the active-page refresh omission. The next failed during compilation because the `/tmp` user quota filled. Neither is a pass. Stopped roots were relocated only after exact content/mode/symlink verification and confirmation that no owned actor remained. The successful retry used the unchanged acceptance command and cases.

These are dirty-working-tree local proofs. No commit, push, source-provenance bypass, advisory waiver, paid provider request, or guessed database migration was performed. The broader release-provenance, upstream-advisory, and supplemental-JEV blockers in the management report remain unchanged. Schema-5 history is preserved but is not available in the new profile.

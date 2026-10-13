# Development

Prerequisites for source development are Rust 1.98.1, Node 22.22.3, pnpm
11.25.0, Python 3, and the platform's Tauri prerequisites when building the
desktop.

The verifier must pass its mutation selftest before its repository result is
trusted. Build and package commands are run through the tracked-source snapshot
guard.

```bash
python3 tools/fleet/cutokyo-gates.py selftest
python3 tools/fleet/cutokyo-gates.py all --root .
cargo fmt --check
python3 tools/scripts/source-snapshot.py -- cargo clippy --workspace --all-targets -- -D warnings
python3 tools/scripts/source-snapshot.py -- cargo test --workspace
cargo deny check licenses bans
pnpm --dir ui check
python3 tools/scripts/source-snapshot.py -- dist plan
```

The packaged native keyboard journey in `pnpm --dir ui test:e2e:tauri` currently
requires Linux, a running Hyprland session with XWayland, and an executable
`hyprctl` on the host runner's `PATH`. The runner resolves that system tool to its
absolute real path in `CUTOKYO_NATIVE_COMPOSITOR_CLI`; the test refuses a missing,
relative, non-regular, or non-executable CLI. Keep the host
`HYPRLAND_INSTANCE_SIGNATURE` and `XDG_RUNTIME_DIR` available to the runner. Only
its scoped compositor subprocess uses the host runtime directory; the application
keeps private HOME and XDG directories. Unsupported compositor/input prerequisites
fail the journey, without skipping it or falling back to global keys or window
focus commands. Run native GUI acceptance without another native runner in parallel.

CI lints once on Linux and builds/tests on Linux, macOS, and Windows. RustSec,
cargo-pup, cargo-machete, and Knip are visible non-gating drift reports. Release
tests produce local or dry-run artifacts only; publishing requires an authorized
tag and all signing credentials.

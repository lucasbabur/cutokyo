#!/usr/bin/env python3
"""Build a test-only Debian bundle, then drive its extracted executable with WDIO.

No source/debug executable is accepted. Each invocation owns its build, assets,
ports, HOME and data. WDIO and all its children stop before those roots disappear.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

UI = Path(__file__).resolve().parents[1]
REPOSITORY = UI.parent


def live_group(group: int) -> bool:
    """Ignore already-exited zombies; never select processes by name or port."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
            if int(fields[2]) == group and fields[0] != "Z":
                return True
        except (FileNotFoundError, ProcessLookupError):
            continue
    return False


def stop_group(group: int) -> None:
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(group, sig)
        except ProcessLookupError:
            return
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if not live_group(group):
                return
            time.sleep(0.05)
    raise RuntimeError("Owned native process group did not stop; retaining test roots")


def run(command: list[str], *, cwd: Path, env: dict[str, str]) -> None:
    print(json.dumps({"command": command, "cwd": str(cwd)}), flush=True)
    process = subprocess.Popen(command, cwd=cwd, env=env, start_new_session=True)
    try:
        code = process.wait()
    finally:
        # WDIO hooks may run before service teardown, or startup may throw. The
        # outer owner is the only place allowed to remove app data/package files.
        stop_group(process.pid)
        process.wait()
    if code:
        raise subprocess.CalledProcessError(code, command)


def reserve_port() -> socket.socket:
    reservation = socket.socket()
    reservation.bind(("127.0.0.1", 0))
    reservation.listen(1)
    return reservation


def build_environment(root: Path) -> dict[str, str]:
    return {
        **os.environ,
        "CARGO_TARGET_DIR": str(root / "target"),
        "CUTOKYO_NATIVE_ASSET_DIR": str(root / "assets"),
    }


def tauri_config(root: Path) -> dict[str, object]:
    return {
        "build": {
            "beforeBuildCommand": "pnpm --dir ui build:e2e:tauri:ui",
            "frontendDist": str(root / "assets"),
        },
    }


def extract_package(root: Path, env: dict[str, str]) -> tuple[Path, Path]:
    packages = list((root / "target/debug/bundle/deb").glob("*.deb"))
    if len(packages) != 1:
        raise RuntimeError(f"Expected one built Debian package, found {len(packages)}")
    package = packages[0]
    destination = root / "package"
    run(["dpkg-deb", "--extract", str(package), str(destination)], cwd=root, env=env)
    application = destination / "usr/bin/cutokyo-desktop"
    if application.is_symlink() or not application.is_file() or not os.access(application, os.X_OK):
        raise RuntimeError("The Debian bundle has no regular executable desktop application")
    return package, application


def runtime_environment(root: Path, evidence: Path, application: Path, port: int) -> dict[str, str]:
    env = dict(os.environ)
    # No inherited operator harness locations, fixture selectors or loader hooks.
    for key in list(env):
        if key.startswith(("CUTOKYO_", "CLAUDE_", "CODEX_", "OPENCODE_", "XDG_")):
            env.pop(key)
    for name in ("home", "config", "data", "cache", "runtime"):
        (root / name).mkdir(mode=0o700)
    env.update({
        "HOME": str(root / "home"),
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_DATA_HOME": str(root / "data"),
        "XDG_CACHE_HOME": str(root / "cache"),
        "XDG_RUNTIME_DIR": str(root / "runtime"),
        "CUTOKYO_DESKTOP_TEST_MODE": "1",
        "CUTOKYO_DESKTOP_TEST_ROOT": str(root),
        "CUTOKYO_DESKTOP_TEST_FIXTURE": "search-resume",
        "CUTOKYO_NATIVE_RESUME_AUDIT": str(root / "resume-audit.txt"),
        "CUTOKYO_NATIVE_APPLICATION": str(application),
        "CUTOKYO_NATIVE_EVIDENCE": str(evidence),
        "CUTOKYO_NATIVE_PORT": str(port),
        "PATH": str(root / "bin") + os.pathsep + env.get("PATH", ""),
    })
    return env


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def interrupted(signum: int, _frame: object) -> None:
    raise InterruptedError(f"Native package runner interrupted by signal {signum}")


def main() -> None:
    signal.signal(signal.SIGTERM, interrupted)
    if sys.platform != "linux":
        raise RuntimeError("Native package journey currently requires Linux; no source-binary fallback")
    evidence_parent = REPOSITORY / "target/native-evidence"
    evidence_parent.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="attempt-", dir=evidence_parent))
    root = Path(tempfile.mkdtemp(prefix="cutokyo-native-test-wdio-"))
    print(f"Native package evidence: {evidence}", flush=True)
    print(f"Isolated native root: {root}", flush=True)
    # Preserve unsuccessful roots and artifacts. Successful roots are removed only
    # after run() has stopped and waited for its owned process group.
    env = build_environment(root)
    run(["cargo", "build", "--manifest-path", str(REPOSITORY / "Cargo.toml"),
         "-p", "fake-harness", "--bin", "fake-resume-harness"], cwd=REPOSITORY, env=env)
    run(["cargo", "tauri", "build", "--debug", "--bundles", "deb", "--features", "native-e2e",
         "--config", json.dumps(tauri_config(root))], cwd=REPOSITORY / "crates/cutokyo-desktop", env=env)
    package, application = extract_package(root, env)
    (root / "bin").mkdir(mode=0o700)
    shutil.copy2(root / "target/debug/fake-resume-harness", root / "bin/claude")
    with reserve_port() as reservation:
        port = reservation.getsockname()[1]
        native_env = runtime_environment(root, evidence, application, port)
        manifest = {
            "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip(),
            "package": str(package), "package_sha256": sha256(package),
            "application": str(application), "application_sha256": sha256(application),
            "asset_directory": str(root / "assets"), "root": str(root), "port": port,
            "launch_origin": "extracted-debian-bundle", "test_only": True,
        }
        with (evidence / "package-launch.json").open("x") as output:
            json.dump(manifest, output, indent=2)
        # Keep the port reserved throughout preparation; never use a fixed/shared
        # port. A subsequent bind collision must fail, not reuse another server.
        reservation.close()
        run(["pnpm", "exec", "wdio", "run", "./wdio.conf.ts"], cwd=UI, env=native_env)
    shutil.copy2(root / "resume-audit.txt", evidence / "resume-audit.txt")
    shutil.rmtree(root)


if __name__ == "__main__":
    main()

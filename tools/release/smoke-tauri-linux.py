#!/usr/bin/env python3
"""Install a Linux Tauri package and prove liveness/readiness/uninstall behavior."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NoReturn

PACKAGE_NAME = re.compile(r"^[a-z0-9][a-z0-9+.-]*$")


def fail(message: str) -> NoReturn:
    print(f"smoke-tauri-linux: {message}", file=sys.stderr)
    raise SystemExit(1)


def run(
    command: list[str],
    *,
    check: bool = True,
    env: dict[str, str] | None = None,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        timeout=120,
        env=env,
        cwd=cwd,
    )
    if check and completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no diagnostic"
        fail(f"command failed with exit {completed.returncode}: {detail[:500]}")
    return completed


def one_debian_package(root: Path) -> Path:
    packages = sorted(path for path in root.rglob("*.deb") if path.is_file())
    if len(packages) != 1:
        fail(f"expected exactly one Debian package, found {len(packages)}")
    return packages[0].resolve()


def package_name(package: Path) -> str:
    name = run(["dpkg-deb", "--field", os.fspath(package), "Package"]).stdout.strip()
    if PACKAGE_NAME.fullmatch(name) is None:
        fail("Debian package has an unsafe or missing package name")
    return name


def installed_binary(name: str) -> Path:
    listed = run(["dpkg-query", "--listfiles", name]).stdout.splitlines()
    candidates = [
        Path(line)
        for line in listed
        if line.startswith("/usr/bin/") and Path(line).is_file()
    ]
    if len(candidates) != 1:
        fail(f"expected one installed /usr/bin executable, found {len(candidates)}")
    binary = candidates[0].resolve()
    if binary.parent != Path("/usr/bin") or not binary.is_file():
        fail("installed executable did not resolve to a regular /usr/bin file")
    return binary


def parse_json(completed: subprocess.CompletedProcess[str], label: str) -> dict[str, object]:
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        fail(f"{label} did not emit one JSON document: {error}")
    if not isinstance(payload, dict):
        fail(f"{label} output is not a JSON object")
    return payload


def remove_package(name: str) -> None:
    run(["sudo", "dpkg", "--remove", name])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()

    package = one_debian_package(args.artifacts.resolve())
    name = package_name(package)
    expected_version = args.expected_version.removeprefix("v")
    existing = run(["dpkg-query", "--show", "--showformat=${db:Status-Abbrev}", name], check=False)
    if existing.returncode == 0:
        fail("refusing to replace or remove an existing package; use a disposable machine/container")
    package_digest = hashlib.sha256(package.read_bytes()).hexdigest()
    with tempfile.TemporaryDirectory(prefix="cutokyo-package-inspect-") as unpacked:
        run(["dpkg-deb", "--extract", os.fspath(package), unpacked])
        binaries = list((Path(unpacked) / "usr/bin").iterdir())
        if len(binaries) != 1 or binaries[0].is_symlink() or not binaries[0].is_file():
            fail("Debian package must contain exactly one regular native executable")
        executable_digest = hashlib.sha256(binaries[0].read_bytes()).hexdigest()
    installed = False
    try:
        installed = True  # A failed dpkg install can still leave unpacked files.
        run(["sudo", "dpkg", "--install", os.fspath(package)])
        binary = installed_binary(name)
        if hashlib.sha256(binary.read_bytes()).hexdigest() != executable_digest:
            fail("installed desktop executable differs from the built package")
        with tempfile.TemporaryDirectory(prefix="cutokyo-desktop-smoke-") as temporary:
            root = Path(temporary)
            environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
            environment.update(
                {
                    "HOME": os.fspath(root / "home"),
                    "XDG_CONFIG_HOME": os.fspath(root / "config"),
                    "XDG_DATA_HOME": os.fspath(root / "data"),
                    "CUTOKYO_CONFIG_FILE": os.fspath(root / "config" / "cutokyo" / "config.toml"),
                    "CUTOKYO_DATA_DIR": os.fspath(root / "data" / "cutokyo"),
                }
            )
            liveness = run(
                [os.fspath(binary), "--cutokyo-probe-liveness"],
                env=environment,
                cwd=root,
            )
            live_payload = parse_json(liveness, "liveness probe")
            if live_payload.get("process_liveness") is not True:
                fail("installed desktop executable did not prove process liveness")
            if live_payload.get("app_version") != expected_version:
                fail("installed desktop executable reported the wrong version")

            readiness = run(
                [os.fspath(binary), "--cutokyo-probe-readiness"],
                check=False,
                env=environment,
                cwd=root,
            )
            ready_payload = parse_json(readiness, "readiness probe")
            if readiness.returncode != 69:
                fail(f"uninitialized readiness probe exited {readiness.returncode}, expected 69")
            if (
                ready_payload.get("process_liveness") is not True
                or ready_payload.get("product_readiness") is not False
                or ready_payload.get("outcome") != "capability_unavailable"
            ):
                fail("readiness probe did not distinguish a live process from an unready product")

            restores = []
            for _ in range(2):
                restored = run([os.fspath(binary), "--cutokyo-uninstall"], env=environment, cwd=root)
                restore = parse_json(restored, "no-state restore")
                if restore != {"managed_state_found": False, "removed": [], "history_preserved": True}:
                    fail("no-state restore did not report an idempotent history-preserving no-op")
                restores.append(restored.returncode)

        remove_package(name)
        installed = False
        if binary.exists():
            fail("desktop executable remained after package uninstall")
        remove_package(name)
        print(
            json.dumps(
                {
                    "binary": binary.name,
                    "gui_activated": False,
                    "liveness_exit": 0,
                    "process_liveness": True,
                    "package": package.name,
                    "package_sha256": package_digest,
                    "installed_executable_sha256": executable_digest,
                    "product_readiness": False,
                    "product_readiness_exit": 69,
                    "first_uninstall_exit": 0,
                    "repeat_uninstall_exit": 0,
                    "no_state_restore_exits": restores,
                    "source_tree_shortcut": False,
                    "version": expected_version,
                },
                sort_keys=True,
            )
        )
        return 0
    finally:
        if installed:
            subprocess.run(
                ["sudo", "dpkg", "--remove", name],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=120,
            )


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Install the generated npm wrapper and launch its packaged native executable."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import NoReturn


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        return


def fail(message: str) -> NoReturn:
    print(f"smoke-npm: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_native_checksum(checksum_path: Path, native_archive: Path) -> str:
    try:
        lines = [
            line.strip()
            for line in checksum_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
    except (OSError, UnicodeError) as error:
        fail(f"could not read native checksum: {error}")
    if len(lines) != 1:
        fail("native checksum must contain exactly one nonempty line")
    match = re.fullmatch(r"([0-9A-Fa-f]{64})\s+[ *]?([^/\\]+)", lines[0])
    if match is None:
        fail("native checksum is not a bounded SHA-256 sidecar")
    expected, filename = match.groups()
    if filename != native_archive.name:
        fail(
            "native checksum names a different artifact: "
            f"expected {native_archive.name}, received {filename}"
        )
    actual = sha256(native_archive)
    if not hmac.compare_digest(expected.lower(), actual):
        fail(
            "native archive checksum mismatch: "
            f"expected {expected.lower()}, received {actual}"
        )
    return actual


def run(
    command: list[str], *, cwd: Path, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        capture_output=True,
        text=True,
        timeout=120,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--npm-package", required=True, type=Path)
    parser.add_argument("--native-archive", required=True, type=Path)
    parser.add_argument("--native-checksum", required=True, type=Path)
    parser.add_argument("--dependency-package", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()

    npm_package = args.npm_package.resolve()
    native_archive = args.native_archive.resolve()
    native_checksum = args.native_checksum.resolve()
    dependency_package = args.dependency_package.resolve()
    required_files = {
        "npm package": npm_package,
        "native archive": native_archive,
        "native checksum": native_checksum,
        "npm dependency package": dependency_package,
    }
    missing = [label for label, path in required_files.items() if not path.is_file()]
    if missing:
        fail("required regular files are missing: " + ", ".join(missing))
    native_digest = verify_native_checksum(native_checksum, native_archive)

    with tempfile.TemporaryDirectory(prefix="cutokyo-npm-smoke-") as temporary:
        root = Path(temporary)
        project = root / "consumer"
        project.mkdir()
        npm_cache = root / "npm-cache"
        npm_cache.mkdir()
        offline_environment = os.environ.copy()
        offline_environment.update(
            {
                "NO_PROXY": "127.0.0.1,localhost",
                "no_proxy": "127.0.0.1,localhost",
                "npm_config_audit": "false",
                "npm_config_cache": os.fspath(npm_cache),
                "npm_config_fund": "false",
                "npm_config_offline": "true",
                "npm_config_registry": "http://127.0.0.1:9/",
                "npm_config_update_notifier": "false",
            }
        )
        cached = run(
            ["npm", "cache", "add", os.fspath(dependency_package)],
            cwd=root,
            env=offline_environment,
        )
        if cached.returncode != 0:
            fail(
                "could not seed the private offline npm cache: "
                f"{cached.stderr.strip()}"
            )
        (project / "package.json").write_text(
            '{"name":"cutokyo-artifact-smoke","private":true}\n', encoding="utf-8"
        )
        install = run(
            [
                "npm",
                "install",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--offline",
                "--omit=dev",
                os.fspath(dependency_package),
                os.fspath(npm_package),
            ],
            cwd=project,
            env=offline_environment,
        )
        if install.returncode != 0:
            fail(f"npm install exited {install.returncode}: {install.stderr.strip()}")

        installed = project / "node_modules" / "cutokyo"
        metadata_path = installed / "package.json"
        if not metadata_path.is_file():
            fail("npm did not install the cutokyo package")

        handler = partial(QuietHandler, directory=os.fspath(native_archive.parent))
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            # cargo-dist bakes release URLs into its generated package. Rebinding only
            # the installed test copy lets the real downloader consume a local dry-run
            # artifact without publishing it or substituting a source-tree executable.
            metadata["artifactDownloadUrls"] = [
                f"http://127.0.0.1:{server.server_port}"
            ]
            metadata_path.write_text(
                json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            postinstall = run(
                ["node", "install.js"], cwd=installed, env=offline_environment
            )
            if postinstall.returncode != 0:
                fail(
                    f"generated postinstall exited {postinstall.returncode}: "
                    f"{postinstall.stderr.strip()}"
                )
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

        launcher = project / "node_modules" / ".bin" / (
            "cutokyo.cmd" if os.name == "nt" else "cutokyo"
        )
        if not launcher.exists():
            fail("npm package did not expose node_modules/.bin/cutokyo")
        environment = offline_environment.copy()
        environment["CUTOKYO_CONFIG_FILE"] = os.fspath(root / "config.toml")
        environment["CUTOKYO_DATA_DIR"] = os.fspath(root / "data")
        launched = run(
            [os.fspath(launcher), "--json", "version"], cwd=project, env=environment
        )
        if launched.returncode != 0:
            fail(f"installed launcher exited {launched.returncode}: {launched.stderr.strip()}")
        try:
            payload = json.loads(launched.stdout)
        except json.JSONDecodeError as error:
            fail(f"installed launcher emitted invalid JSON: {error}")
        expected = args.expected_version.removeprefix("v")
        actual = payload.get("data", {}).get("app_version")
        if payload.get("ok") is not True or actual != expected:
            fail(f"expected installed version {expected!r}, received {actual!r}")

        native_name = "cutokyo.exe" if os.name == "nt" else "cutokyo"
        native_candidates = [
            path
            for path in installed.rglob(native_name)
            if path.is_file() and path.name == native_name and path.parent.name != ".bin"
        ]
        if len(native_candidates) != 1:
            fail(f"expected one installed native executable, found {len(native_candidates)}")
        native = native_candidates[0].resolve()
        try:
            native.relative_to(installed.resolve())
        except ValueError:
            fail("installed native executable resolves outside the npm package")

        print(
            json.dumps(
                {
                    "npm_package": npm_package.name,
                    "npm_sha256": sha256(npm_package),
                    "native_archive": native_archive.name,
                    "native_checksum": native_checksum.name,
                    "native_sha256": native_digest,
                    "dependency_package": dependency_package.name,
                    "dependency_sha256": sha256(dependency_package),
                    "network_mode": "offline-private-cache-and-loopback-artifact-server",
                    "launcher": "node_modules/.bin/cutokyo",
                    "native_location": native.relative_to(project).as_posix(),
                    "source_tree_shortcut": False,
                    "version": actual,
                },
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

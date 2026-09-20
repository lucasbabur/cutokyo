#!/usr/bin/env python3
"""Execute Cutokyo from a cargo-dist archive, never from the source tree."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path
from typing import NoReturn

ARCHIVE_SUFFIXES = (".tar.gz", ".tar.xz", ".tar.zst", ".zip")


def fail(message: str) -> NoReturn:
    print(f"smoke-dist: {message}", file=sys.stderr)
    raise SystemExit(1)


def archive_candidates(root: Path, target: str) -> list[Path]:
    result: list[Path] = []
    for path in root.rglob("*"):
        if not path.is_file() or target not in path.name:
            continue
        if path.name.endswith(ARCHIVE_SUFFIXES):
            result.append(path)
    return sorted(result)


def extract(archive: Path, destination: Path) -> None:
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as package:
            package.extractall(destination)
        return
    if archive.name.endswith((".tar.gz", ".tar.xz")):
        with tarfile.open(archive, mode="r:*") as package:
            package.extractall(destination, filter="data")
        return
    if archive.name.endswith(".tar.zst"):
        tar = shutil.which("tar")
        if tar is None:
            fail("the host has no tar executable for a .tar.zst package")
        completed = subprocess.run(
            [tar, "--extract", "--file", os.fspath(archive), "--directory", os.fspath(destination)],
            check=False,
        )
        if completed.returncode != 0:
            fail(f"could not extract {archive.name}")
        return
    fail(f"unsupported archive {archive.name}")


def find_binary(root: Path, target: str) -> Path:
    expected_name = "cutokyo.exe" if "windows" in target else "cutokyo"
    matches = [
        path
        for path in root.rglob(expected_name)
        if path.is_file() and not path.is_symlink()
    ]
    if len(matches) != 1:
        fail(f"expected exactly one packaged {expected_name}, found {len(matches)}")
    binary = matches[0].resolve()
    try:
        binary.relative_to(root.resolve())
    except ValueError:
        fail("packaged executable resolves outside the extraction directory")
    return binary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()

    archives = archive_candidates(args.artifacts, args.target)
    if not archives:
        fail(f"no cargo-dist archive for {args.target}")

    expected = args.expected_version.removeprefix("v")
    failures: list[str] = []
    for archive in archives:
        with tempfile.TemporaryDirectory(prefix="cutokyo-dist-smoke-") as temporary:
            extraction = Path(temporary)
            try:
                extract(archive, extraction)
                binary = find_binary(extraction, args.target)
                completed = subprocess.run(
                    [os.fspath(binary), "--json", "version"],
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=20,
                    cwd=extraction,
                )
                if completed.returncode != 0:
                    failures.append(f"{archive.name}: exit {completed.returncode}")
                    continue
                payload = json.loads(completed.stdout)
                actual = payload.get("data", {}).get("app_version")
                if payload.get("ok") is not True or actual != expected:
                    failures.append(
                        f"{archive.name}: expected version {expected!r}, received {actual!r}"
                    )
                    continue
                print(
                    json.dumps(
                        {
                            "archive": archive.name,
                            "binary": binary.name,
                            "source_tree_shortcut": False,
                            "version": actual,
                        },
                        sort_keys=True,
                    )
                )
                return 0
            except (OSError, ValueError, json.JSONDecodeError, subprocess.TimeoutExpired) as error:
                failures.append(f"{archive.name}: {error}")

    fail("no archive passed installed-binary smoke: " + "; ".join(failures))


if __name__ == "__main__":
    raise SystemExit(main())

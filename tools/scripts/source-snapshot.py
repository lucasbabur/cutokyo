#!/usr/bin/env python3
"""Guard build and package commands against tracked-source rewrites."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

SNAPSHOT_SCHEMA_VERSION = 1


def fail(message: str, code: int = 2) -> NoReturn:
    print(f"source-snapshot: {message}", file=sys.stderr)
    raise SystemExit(code)


def repository_root(explicit: Path | None) -> Path:
    candidate = explicit.resolve() if explicit is not None else Path.cwd().resolve()
    completed = subprocess.run(
        ["git", "-C", os.fspath(candidate), "rev-parse", "--show-toplevel"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        fail("--root is not a Git worktree" if explicit else "current directory is not inside a Git worktree")
    root = Path(completed.stdout.strip()).resolve()
    if explicit is not None and candidate != root:
        fail("--root must name the exact Git worktree root")
    return root


def tracked_snapshot(root: Path) -> dict[str, str]:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), "ls-files", "-z"],
        check=False,
        capture_output=True,
    )
    if completed.returncode != 0:
        fail("git ls-files failed")
    result: dict[str, str] = {}
    for raw_name in completed.stdout.split(b"\0"):
        if not raw_name:
            continue
        relative = os.fsdecode(raw_name)
        path = root / relative
        if path.is_symlink():
            payload = b"symlink\0" + os.fsencode(os.readlink(path))
        elif path.is_file():
            payload = b"file\0" + path.read_bytes()
        else:
            payload = b"missing\0"
        result[relative] = hashlib.sha256(payload).hexdigest()
    return result


def changed_paths(before: dict[str, str], after: dict[str, str]) -> list[str]:
    names = sorted(set(before) | set(after))
    return [name for name in names if before.get(name) != after.get(name)]


def tracked_worktree_changes(root: Path) -> list[str]:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), "diff", "--no-renames", "--name-only", "-z", "HEAD", "--"],
        check=False,
        capture_output=True,
    )
    if completed.returncode != 0:
        fail("could not compare tracked source with HEAD")
    return sorted(
        {
            os.fsdecode(raw_name)
            for raw_name in completed.stdout.split(b"\0")
            if raw_name
        }
    )


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def capture(path: Path, root: Path) -> None:
    write_json(
        path,
        {
            "schema_version": SNAPSHOT_SCHEMA_VERSION,
            "tracked": tracked_snapshot(root),
        },
    )


def load_capture(path: Path) -> dict[str, str]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"could not read snapshot: {error}")
    if not isinstance(value, dict) or value.get("schema_version") != SNAPSHOT_SCHEMA_VERSION:
        fail("snapshot schema is invalid")
    tracked = value.get("tracked")
    if not isinstance(tracked, dict) or not all(
        isinstance(name, str) and isinstance(digest, str)
        for name, digest in tracked.items()
    ):
        fail("snapshot tracked-file map is invalid")
    return tracked


def emit_evidence(evidence: dict[str, object], manifest: Path | None) -> None:
    if manifest is not None:
        write_json(manifest, evidence)
    print(json.dumps(evidence, sort_keys=True), file=sys.stderr)


def check_capture(root: Path, snapshot_path: Path, manifest: Path | None) -> int:
    before = load_capture(snapshot_path)
    after = tracked_snapshot(root)
    repository_changes = tracked_worktree_changes(root)
    changed = sorted(set(changed_paths(before, after)) | set(repository_changes))
    evidence: dict[str, object] = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "mode": "verify",
        "tracked_files": len(before),
        "source_unchanged": not changed,
        "changed_paths": changed,
        "repository_clean": not repository_changes,
        "preexisting_changes": repository_changes,
    }
    emit_evidence(evidence, manifest)
    return 86 if changed else 0


def command_directory(root: Path, requested: Path | None) -> Path:
    directory = root if requested is None else (root / requested).resolve()
    try:
        directory.relative_to(root)
    except ValueError:
        fail("--cwd must resolve inside the repository")
    if not directory.is_dir():
        fail("--cwd must name an existing repository directory")
    return directory


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run a command and verify that tracked source remains byte-identical"
    )
    parser.add_argument(
        "--root",
        type=Path,
        help="exact Git worktree root; use this when the caller's cwd is unrelated",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="optional JSON evidence path (outside tracked source is recommended)",
    )
    parser.add_argument(
        "--cwd",
        type=Path,
        help="run inside this repository-relative directory (defaults to the root)",
    )
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument(
        "--capture",
        type=Path,
        help="write a pre-build tracked-source snapshot and exit",
    )
    modes.add_argument(
        "--verify",
        type=Path,
        help="compare a previously captured snapshot and exit 86 on rewrite",
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    root = repository_root(args.root)

    if args.capture is not None:
        if args.command:
            fail("--capture does not accept a command")
        preexisting_changes = tracked_worktree_changes(root)
        snapshot = tracked_snapshot(root)
        if preexisting_changes:
            emit_evidence(
                {
                    "schema_version": SNAPSHOT_SCHEMA_VERSION,
                    "mode": "capture",
                    "tracked_files": len(snapshot),
                    "source_unchanged": False,
                    "changed_paths": preexisting_changes,
                    "repository_clean": False,
                    "preexisting_changes": preexisting_changes,
                },
                args.manifest,
            )
            return 86
        capture(args.capture, root)
        emit_evidence(
            {
                "schema_version": SNAPSHOT_SCHEMA_VERSION,
                "mode": "capture",
                "tracked_files": len(snapshot),
                "source_unchanged": True,
                "changed_paths": [],
                "repository_clean": True,
                "preexisting_changes": [],
            },
            args.manifest,
        )
        return 0
    if args.verify is not None:
        if args.command:
            fail("--verify does not accept a command")
        return check_capture(root, args.verify, args.manifest)

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        fail("a command is required after --")

    directory = command_directory(root, args.cwd)
    before = tracked_snapshot(root)
    preexisting_changes = tracked_worktree_changes(root)
    if preexisting_changes:
        emit_evidence(
            {
                "schema_version": SNAPSHOT_SCHEMA_VERSION,
                "mode": "command",
                "command": command,
                "command_directory": directory.relative_to(root).as_posix() or ".",
                "command_executed": False,
                "command_exit_code": None,
                "tracked_files": len(before),
                "source_unchanged": False,
                "changed_paths": preexisting_changes,
                "repository_clean": False,
                "preexisting_changes": preexisting_changes,
            },
            args.manifest,
        )
        return 86
    try:
        completed = subprocess.run(command, cwd=directory, check=False)
    except OSError as error:
        fail(f"could not execute command: {error}", 127)
    after = tracked_snapshot(root)
    repository_changes = tracked_worktree_changes(root)
    changed = sorted(set(changed_paths(before, after)) | set(repository_changes))
    evidence = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "mode": "command",
        "command": command,
        "command_directory": directory.relative_to(root).as_posix() or ".",
        "command_executed": True,
        "command_exit_code": completed.returncode,
        "tracked_files": len(before),
        "source_unchanged": not changed,
        "changed_paths": changed,
        "repository_clean": not repository_changes,
        "preexisting_changes": [],
    }
    emit_evidence(evidence, args.manifest)
    if changed:
        return 86
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Guard build and package commands against tracked-source rewrites."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

SNAPSHOT_SCHEMA_VERSION = 3
SOURCE_CHANGED_EXIT = 86
POLL_INTERVAL_SECONDS = 0.01


@dataclass(frozen=True)
class RepositoryState:
    """Immutable Git and tracked-worktree state used by one proof."""

    head: str
    index_sha256: str
    tracked: dict[str, str]


class CaptureError(ValueError):
    """Raised when a split snapshot is malformed or no longer applicable."""


def fail(message: str, code: int = 2) -> NoReturn:
    print(f"source-snapshot: {message}", file=sys.stderr)
    raise SystemExit(code)


def run_git(root: Path, arguments: list[str], failure: str) -> bytes:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), *arguments],
        check=False,
        capture_output=True,
    )
    if completed.returncode != 0:
        fail(failure)
    return completed.stdout


def repository_root(explicit: Path | None) -> Path:
    candidate = explicit.resolve() if explicit is not None else Path.cwd().resolve()
    completed = subprocess.run(
        ["git", "-C", os.fspath(candidate), "rev-parse", "--show-toplevel"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = "--root is not a Git worktree" if explicit else "current directory is not inside a Git worktree"
        fail(detail)
    root = Path(completed.stdout.strip()).resolve()
    if explicit is not None and candidate != root:
        fail("--root must name the exact Git worktree root")
    return root


def path_is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def external_path(path: Path | None, root: Path, option: str) -> Path | None:
    if path is None:
        return None
    if path.is_symlink():
        fail(f"{option} may not be a symbolic link")
    resolved = path.resolve(strict=False)
    if path_is_within(resolved, root):
        fail(f"{option} must resolve outside the Git worktree")
    for git_directory in ("--git-dir", "--git-common-dir"):
        raw = run_git(root, ["rev-parse", "--path-format=absolute", git_directory], "could not locate Git metadata")
        if path_is_within(resolved, Path(os.fsdecode(raw).strip()).resolve()):
            fail(f"{option} may not be inside Git metadata")
    if option != "--verify" and resolved.exists():
        fail(f"{option} refuses to replace existing evidence")
    return resolved


def tracked_names(root: Path) -> list[str]:
    raw = run_git(root, ["ls-files", "-z"], "git ls-files failed")
    return sorted(os.fsdecode(name) for name in raw.split(b"\0") if name)


def file_digest(path: Path) -> str:
    if path.is_symlink():
        payload = b"symlink\0" + os.fsencode(os.readlink(path))
    elif path.is_file():
        payload = b"file\0" + path.read_bytes()
    else:
        payload = b"missing\0"
    return hashlib.sha256(payload).hexdigest()


def tracked_snapshot(root: Path, names: list[str] | None = None) -> dict[str, str]:
    selected = tracked_names(root) if names is None else names
    return {relative: file_digest(root / relative) for relative in selected}


def change_time(path: Path, metadata: os.stat_result) -> int:
    if os.name != "nt":
        return metadata.st_ctime_ns
    # Windows st_ctime is creation time, not the NTFS change timestamp. Query
    # FileBasicInfo so a write/restore (even with restored mtime) stays observable.
    import ctypes
    from ctypes import wintypes

    class BasicInfo(ctypes.Structure):
        _fields_ = [(name, ctypes.c_longlong) for name in
                    ("creation", "access", "write", "change")] + [("attributes", wintypes.DWORD)]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                   ctypes.c_void_p, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.CreateFileW(str(path), 0x80, 7, None, 3, 0x02200000, None)
    if handle == wintypes.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        info = BasicInfo()
        if not kernel.GetFileInformationByHandleEx(handle, 0, ctypes.byref(info), ctypes.sizeof(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        return info.change
    finally:
        kernel.CloseHandle(handle)


def tracked_fingerprints(root: Path, names: list[str]) -> dict[str, tuple[int, int, int, int, int] | None]:
    fingerprints: dict[str, tuple[int, int, int, int, int] | None] = {}
    for relative in names:
        path = root / relative
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            fingerprints[relative] = None
            continue
        fingerprints[relative] = (
            metadata.st_mode,
            metadata.st_size,
            metadata.st_mtime_ns,
            change_time(path, metadata),
            getattr(metadata, "st_ino", 0),
        )
    return fingerprints


def changed_fingerprints(
    before: dict[str, tuple[int, int, int, int, int] | None],
    after: dict[str, tuple[int, int, int, int, int] | None],
) -> set[str]:
    return {name for name in set(before) | set(after) if before.get(name) != after.get(name)}


def changed_paths(before: dict[str, str], after: dict[str, str]) -> list[str]:
    return sorted(name for name in set(before) | set(after) if before.get(name) != after.get(name))


def head_oid(root: Path) -> str:
    return run_git(root, ["rev-parse", "--verify", "HEAD"], "could not resolve HEAD").decode("ascii").strip()


def index_sha256(root: Path) -> str:
    raw = run_git(root, ["ls-files", "--stage", "-z"], "could not fingerprint the Git index")
    return hashlib.sha256(raw).hexdigest()


def repository_state(root: Path, names: list[str] | None = None) -> RepositoryState:
    return RepositoryState(
        head=head_oid(root),
        index_sha256=index_sha256(root),
        tracked=tracked_snapshot(root, names),
    )


def tracked_worktree_changes(root: Path) -> list[str]:
    raw = run_git(
        root,
        ["diff", "--no-renames", "--name-only", "-z", "HEAD", "--"],
        "could not compare tracked source with HEAD",
    )
    return sorted({os.fsdecode(name) for name in raw.split(b"\0") if name})


def canonical_json(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode("utf-8")


def capture_digest(value: dict[str, object]) -> str:
    unsigned = {key: item for key, item in value.items() if key != "capture_sha256"}
    return hashlib.sha256(canonical_json(unsigned)).hexdigest()


def durable_directory(directory: Path) -> None:
    if os.name == "nt":
        return
    descriptor = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_capture_exclusive(path: Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() or path.is_symlink():
        fail("--capture refuses to replace existing evidence")
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        if os.name != "nt":
            os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            descriptor = -1
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError:
            fail("--capture refuses to replace existing evidence")
        durable_directory(path.parent)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def root_binding(root: Path) -> str:
    git_common = run_git(
        root,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        "could not resolve the Git common directory",
    ).decode("utf-8").strip()
    identity = f"{root}\0{Path(git_common).resolve()}".encode()
    return hashlib.sha256(identity).hexdigest()


def git_movement_fingerprints(root: Path) -> dict[str, tuple[int, int, int, int, int] | None]:
    paths = {}
    for name in ("HEAD", "index", "logs/HEAD"):
        raw = run_git(root, ["rev-parse", "--path-format=absolute", "--git-path", name], "could not locate Git state")
        path = Path(os.fsdecode(raw).strip())
        paths[name] = tracked_fingerprints(path.parent, [path.name])[path.name]
    return paths


def make_capture(root: Path, state: RepositoryState) -> dict[str, object]:
    value: dict[str, object] = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "root_binding": root_binding(root),
        "head": state.head,
        "index_sha256": state.index_sha256,
        "tracked": state.tracked,
        "fingerprints": tracked_fingerprints(root, list(state.tracked)),
        "git_fingerprints": git_movement_fingerprints(root),
    }
    value["capture_sha256"] = capture_digest(value)
    return value


def load_capture(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise CaptureError(f"could not read snapshot: {error}") from error
    required = {
        "schema_version",
        "root_binding",
        "head",
        "index_sha256",
        "tracked",
        "fingerprints",
        "git_fingerprints",
        "capture_sha256",
    }
    if not isinstance(value, dict) or set(value) != required:
        raise CaptureError("snapshot structure is invalid")
    if value.get("schema_version") != SNAPSHOT_SCHEMA_VERSION:
        raise CaptureError("snapshot schema is invalid")
    for field in ("root_binding", "index_sha256", "capture_sha256"):
        item = value.get(field)
        if not isinstance(item, str) or len(item) != 64 or any(character not in "0123456789abcdef" for character in item):
            raise CaptureError(f"snapshot {field} is invalid")
    head = value.get("head")
    if not isinstance(head, str) or len(head) not in (40, 64) or any(character not in "0123456789abcdef" for character in head):
        raise CaptureError("snapshot head is invalid")
    tracked = value.get("tracked")
    if not isinstance(tracked, dict) or not all(
        isinstance(name, str)
        and isinstance(digest, str)
        and len(digest) == 64
        and all(character in "0123456789abcdef" for character in digest)
        for name, digest in tracked.items()
    ):
        raise CaptureError("snapshot tracked-file map is invalid")
    for field, keys in (("fingerprints", set(tracked)),
                        ("git_fingerprints", {"HEAD", "index", "logs/HEAD"})):
        fingerprints = value.get(field)
        if not isinstance(fingerprints, dict) or set(fingerprints) != keys or not all(
            fingerprint is None or (isinstance(fingerprint, list) and len(fingerprint) == 5
                                    and all(type(number) is int for number in fingerprint))
            for fingerprint in fingerprints.values()
        ):
            raise CaptureError(f"snapshot {field} are invalid")
    if value["capture_sha256"] != capture_digest(value):
        raise CaptureError("snapshot integrity digest does not match")
    return value


def emit_evidence(evidence: dict[str, object], manifest: Path | None) -> None:
    if manifest is not None:
        write_capture_exclusive(manifest, evidence)
    print(json.dumps(evidence, sort_keys=True), file=sys.stderr)


def check_capture(root: Path, snapshot_path: Path, manifest: Path | None) -> int:
    capture_error: str | None = None
    try:
        saved = load_capture(snapshot_path)
    except CaptureError as error:
        capture_error = str(error)
        saved = {}
    current = repository_state(root)
    repository_changes = tracked_worktree_changes(root)
    saved_tracked = saved.get("tracked")
    tracked = saved_tracked if isinstance(saved_tracked, dict) else {}
    changed = changed_paths(tracked, current.tracked) if saved else []
    saved_fingerprints = saved.get("fingerprints")
    if saved and isinstance(saved_fingerprints, dict):
        fingerprints = {name: tuple(value) if value is not None else None
                        for name, value in saved_fingerprints.items()}
        changed = sorted(set(changed) | changed_fingerprints(
            fingerprints, tracked_fingerprints(root, list(tracked))))
    state_changes: list[str] = []
    if saved:
        if saved.get("root_binding") != root_binding(root):
            state_changes.append("repository-binding")
        if saved.get("head") != current.head:
            state_changes.append("HEAD")
        if saved.get("index_sha256") != current.index_sha256:
            state_changes.append("index")
    saved_git = saved.get("git_fingerprints")
    if isinstance(saved_git, dict):
        fingerprints = {name: tuple(value) if value is not None else None
                        for name, value in saved_git.items()}
        state_changes = sorted(set(state_changes) | changed_fingerprints(
            fingerprints, git_movement_fingerprints(root)))
    if capture_error is not None:
        state_changes.append("capture-integrity")
    changed = sorted(set(changed) | set(repository_changes))
    unchanged = not changed and not state_changes
    evidence: dict[str, object] = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "mode": "verify",
        "tracked_files": len(tracked),
        "source_unchanged": unchanged,
        "changed_paths": changed,
        "repository_clean": not repository_changes,
        "preexisting_changes": repository_changes,
        "state_changes": state_changes,
        "capture_error": capture_error,
        "head": current.head,
        "index_sha256": current.index_sha256,
    }
    emit_evidence(evidence, manifest)
    return 0 if unchanged else SOURCE_CHANGED_EXIT


def command_directory(root: Path, requested: Path | None) -> Path:
    directory = root if requested is None else (root / requested).resolve()
    if not path_is_within(directory, root):
        fail("--cwd must resolve inside the repository")
    if not directory.is_dir():
        fail("--cwd must name an existing repository directory")
    return directory


def state_changes(before: RepositoryState, after: RepositoryState) -> list[str]:
    changed: list[str] = []
    if before.head != after.head:
        changed.append("HEAD")
    if before.index_sha256 != after.index_sha256:
        changed.append("index")
    return changed


def run_guarded_command(
    root: Path,
    directory: Path,
    command: list[str],
    manifest: Path | None,
) -> int:
    names = tracked_names(root)
    before = repository_state(root, names)
    baseline_fingerprints = tracked_fingerprints(root, names)
    baseline_git = git_movement_fingerprints(root)
    preexisting_changes = tracked_worktree_changes(root)
    base_evidence: dict[str, object] = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "mode": "command",
        "command": command,
        "command_directory": directory.relative_to(root).as_posix() or ".",
        "tracked_files": len(before.tracked),
        "head": before.head,
        "index_sha256": before.index_sha256,
    }
    if preexisting_changes:
        emit_evidence(
            {
                **base_evidence,
                "command_executed": False,
                "command_exit_code": None,
                "source_unchanged": False,
                "changed_paths": preexisting_changes,
                "temporary_changed_paths": [],
                "state_changes": [],
                "repository_clean": False,
                "preexisting_changes": preexisting_changes,
            },
            manifest,
        )
        return SOURCE_CHANGED_EXIT

    try:
        executable = shutil.which(command[0]) or command[0]
        windows_batch = os.name == "nt" and Path(executable).suffix.lower() in {".cmd", ".bat"}
        invocation = subprocess.list2cmdline([executable, *command[1:]]) if windows_batch else command
        process = subprocess.Popen(invocation, cwd=directory, shell=windows_batch)
    except OSError as error:
        fail(f"could not execute command: {error}", 127)

    observed_paths: set[str] = set()
    observed_state_changes: set[str] = set()
    next_git_check = 0.0
    while process.poll() is None:
        observed_paths.update(
            changed_fingerprints(
                baseline_fingerprints,
                tracked_fingerprints(root, names),
            )
        )
        now = time.monotonic()
        if now >= next_git_check:
            current_head = head_oid(root)
            current_index = index_sha256(root)
            if current_head != before.head:
                observed_state_changes.add("HEAD")
            if current_index != before.index_sha256:
                observed_state_changes.add("index")
            next_git_check = now + 0.05
        time.sleep(POLL_INTERVAL_SECONDS)
    command_exit_code = process.wait()

    observed_paths.update(
        changed_fingerprints(
            baseline_fingerprints,
            tracked_fingerprints(root, names),
        )
    )
    after = repository_state(root)
    observed_state_changes.update(state_changes(before, after))
    observed_state_changes.update(changed_fingerprints(baseline_git, git_movement_fingerprints(root)))
    repository_changes = tracked_worktree_changes(root)
    final_changes = changed_paths(before.tracked, after.tracked)
    changed = sorted(set(final_changes) | set(repository_changes) | observed_paths)
    state_change_list = sorted(observed_state_changes)
    unchanged = not changed and not state_change_list
    evidence = {
        **base_evidence,
        "command_executed": True,
        "command_exit_code": command_exit_code,
        "source_unchanged": unchanged,
        "changed_paths": changed,
        "temporary_changed_paths": sorted(observed_paths - set(final_changes) - set(repository_changes)),
        "state_changes": state_change_list,
        "repository_clean": not repository_changes,
        "preexisting_changes": [],
    }
    emit_evidence(evidence, manifest)
    if not unchanged:
        return SOURCE_CHANGED_EXIT
    return command_exit_code


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run a command and prove that tracked source and Git state remain unchanged"
    )
    parser.add_argument(
        "--root",
        type=Path,
        help="exact Git worktree root; use this when the caller's cwd is unrelated",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="optional JSON evidence path; it must resolve outside the worktree",
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
        help="write an immutable-state-bound snapshot outside the worktree and exit",
    )
    modes.add_argument(
        "--verify",
        type=Path,
        help="verify an external snapshot and exit 86 on source or Git-state movement",
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    root = repository_root(args.root)
    manifest = external_path(args.manifest, root, "--manifest")
    capture_path = external_path(args.capture, root, "--capture")
    verify_path = external_path(args.verify, root, "--verify")
    if manifest is not None and manifest in (capture_path, verify_path):
        fail("--manifest must differ from the capture evidence path")

    if capture_path is not None:
        if args.command:
            fail("--capture does not accept a command")
        state = repository_state(root)
        preexisting_changes = tracked_worktree_changes(root)
        if preexisting_changes:
            emit_evidence(
                {
                    "schema_version": SNAPSHOT_SCHEMA_VERSION,
                    "mode": "capture",
                    "tracked_files": len(state.tracked),
                    "source_unchanged": False,
                    "changed_paths": preexisting_changes,
                    "repository_clean": False,
                    "preexisting_changes": preexisting_changes,
                    "state_changes": [],
                    "head": state.head,
                    "index_sha256": state.index_sha256,
                },
                manifest,
            )
            return SOURCE_CHANGED_EXIT
        write_capture_exclusive(capture_path, make_capture(root, state))
        emit_evidence(
            {
                "schema_version": SNAPSHOT_SCHEMA_VERSION,
                "mode": "capture",
                "tracked_files": len(state.tracked),
                "source_unchanged": True,
                "changed_paths": [],
                "repository_clean": True,
                "preexisting_changes": [],
                "state_changes": [],
                "head": state.head,
                "index_sha256": state.index_sha256,
            },
            manifest,
        )
        return 0
    if verify_path is not None:
        if args.command:
            fail("--verify does not accept a command")
        return check_capture(root, verify_path, manifest)

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        fail("a command is required after --")
    directory = command_directory(root, args.cwd)
    return run_guarded_command(root, directory, command, manifest)


if __name__ == "__main__":
    raise SystemExit(main())

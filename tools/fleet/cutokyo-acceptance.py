#!/usr/bin/env python3
"""Run the pinned Cutokyo acceptance contract without changing its commands.

Filtered Rust invocations need complete, consistent libtest results and at least
one passed test. Exit 86 means execution evidence is insufficient, 87 means the
contract differs from the approved Git blob, and 88 means evidence capture failed.
With --log-dir, command output goes directly to durable files while it runs; an
unfinished report stays incomplete even if the runner is killed. Without it, JSON
contains the captured diagnostics, but cannot survive an uncatchable termination.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path

TEST_EVIDENCE_EXIT = 86
CONTRACT_EXIT = 87
EVIDENCE_EXIT = 88
USAGE_EXIT = 2

CONTRACT_PATH = ".claude/fleets/20260919-cutokyo-v01/review.json"
# Approval anchor, deliberately not a branch, CLI option, or environment override.
APPROVED_REVISION = "1db95d8659d31df8cea6933a994d9a196205bf8e"
APPROVED_BLOB = "4f43bfe9c2e8818871a981ac8562545af58e342d"
APPROVED_REFERENCE = f"{APPROVED_REVISION}:{CONTRACT_PATH}"

RUNNING = re.compile(r"^running (\d+) tests?$")
RESULT = re.compile(
    r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out(?:; finished in .*)?$"
)

# Flags that consume the following argument, so it is never a test-name filter.
VALUE_FLAGS = {
    "-p", "--package", "--test", "--bin", "--example", "--bench", "--features",
    "--manifest-path", "--target", "--target-dir", "--profile", "--jobs", "-j",
    "--exclude", "--config", "--color", "--message-format",
}


@dataclass
class Segment:
    index: int
    command: str
    is_filtered_cargo_test: bool = False
    filter: str | None = None
    exit_code: int | None = None
    tests_run: int = 0
    tests_passed: int = 0
    tests_failed: int = 0
    tests_ignored: int = 0
    test_targets: int = 0
    test_results: int = 0
    skipped: bool = False
    state: str = "pending"
    log: str | None = None
    output: str | None = None
    findings: list[str] = field(default_factory=list)


def split_segments(command: str) -> list[str]:
    """Split the contract's top-level ` && ` separators without rewriting bytes."""
    parts: list[str] = []
    buf: list[str] = []
    quote: str | None = None
    i = 0
    while i < len(command):
        ch = command[i]
        if quote:
            buf.append(ch)
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in ("'", '"'):
            quote = ch
            buf.append(ch)
            i += 1
            continue
        if command.startswith(" && ", i):
            parts.append("".join(buf))
            buf = []
            i += 4
            continue
        buf.append(ch)
        i += 1
    if quote:
        raise ValueError("unbalanced quote in recorded command")
    parts.append("".join(buf))
    return parts


def classify(segment: str) -> tuple[bool, str | None]:
    """Return (is cargo test, explicit test-name filter)."""
    tokens = shlex.split(segment)
    if len(tokens) < 2 or tokens[0] != "cargo" or tokens[1] != "test":
        return False, None
    rest = tokens[2:]
    if "--" in rest:
        rest = rest[: rest.index("--")]
    i = 0
    while i < len(rest):
        token = rest[i]
        if token in VALUE_FLAGS:
            i += 2
            continue
        if token.startswith("-"):
            i += 1
            continue
        return True, token
    return True, None


def parse_counts(output: str, seg: Segment) -> None:
    """Pair each target's selection count with its result, not just global sums."""
    pending: int | None = None
    for line in output.splitlines():
        running = RUNNING.fullmatch(line)
        result = RESULT.fullmatch(line)
        if running:
            if pending is not None:
                seg.findings.append("test target is missing its result")
            pending = int(running[1])
            seg.test_targets += 1
            seg.tests_run += pending
        elif result:
            passed, failed, ignored, measured = map(int, result.group(2, 3, 4, 5))
            seg.test_results += 1
            seg.tests_passed += passed
            seg.tests_failed += failed
            seg.tests_ignored += ignored
            if pending is None or pending != passed + failed + ignored + measured:
                seg.findings.append("test result does not match its target's selected count")
            if (result[1] == "ok") != (failed == 0):
                seg.findings.append("test result status contradicts its failure count")
            pending = None
        elif line.startswith("test result:"):
            seg.findings.append("unrecognized test result")
    if pending is not None:
        seg.findings.append("test target is missing its result")
    if not seg.test_targets or not seg.test_results:
        seg.findings.append("no complete parsed test execution")
    if seg.tests_failed:
        seg.findings.append(f"{seg.tests_failed} parsed test(s) failed")
    if not seg.tests_passed:
        seg.findings.append("no test passed; selected or ignored tests are not successful execution")


def git(root: Path, *args: str) -> bytes:
    return subprocess.run(
        ["git", "-C", str(root), *args], check=True, capture_output=True,
    ).stdout


def source_identity(root: Path) -> dict:
    return {
        "revision": git(root, "rev-parse", "HEAD").decode().strip(),
        "tracked_status": git(root, "status", "--porcelain=v1", "--untracked-files=no").decode(),
        "tracked_diff_sha256": hashlib.sha256(git(root, "diff", "--binary", "HEAD")).hexdigest(),
    }


def environment_identity(root: Path, env: dict) -> dict:
    # Do not dump the inherited environment: it can contain credentials. This is
    # execution context, not a claim of a hermetic/reproducible environment.
    identity = {
        "hostname": platform.node(),
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "python_executable": sys.executable,
        "root": str(root),
        "executables": {
            name: shutil.which(name, path=env.get("PATH"))
            for name in ("bash", "git", "cargo", "rustc", "node", "pnpm", "dist")
        },
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    identity["id"] = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    return identity


def approved_contract(root: Path, review: Path) -> tuple[dict, dict]:
    approved = git(root, "show", APPROVED_REFERENCE)
    blob = hashlib.sha1(f"blob {len(approved)}\0".encode() + approved).hexdigest()
    identity = {
        "reference": APPROVED_REFERENCE,
        "git_blob": APPROVED_BLOB,
        "sha256": hashlib.sha256(approved).hexdigest(),
        "path": str(review),
        "verified": False,
    }
    if blob != APPROVED_BLOB:
        raise ValueError("approved reference did not resolve to the pinned contract blob")
    # An alternate --review cannot hide a changed contract in the selected tree.
    if (root / CONTRACT_PATH).read_bytes() != approved or review.read_bytes() != approved:
        raise ValueError("selected worktree acceptance contract differs from the approved Git blob")
    identity["verified"] = True
    return json.loads(approved), identity


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def save_report(path: Path | None, report: dict) -> None:
    if path is None:
        return
    # Atomic replacement keeps the previous incomplete snapshot valid if killed.
    fd, name = tempfile.mkstemp(prefix=f".{path.name}-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(report, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(name):
            os.unlink(name)


class Interrupted(Exception):
    def __init__(self, signum: int):
        self.signum = signum


def interrupt(signum: int, _frame: object) -> None:
    raise Interrupted(signum)


def execute_segment(seg: Segment, root: Path, env: dict, log_dir: Path | None) -> None:
    # Direct file descriptors, not capture_output: partial bytes survive a killed
    # runner. Child-side buffering is outside our control.
    stream = Path(seg.log).open("x+b", buffering=0) if seg.log else tempfile.TemporaryFile()
    proc = None
    output_start = 0
    try:
        if log_dir:
            stream.write(f"$ {seg.command}\n\n".encode())
            os.fsync(stream.fileno())
        output_start = stream.tell()
        proc = subprocess.Popen(
            ["bash", "-c", seg.command], cwd=root, env=env,
            stdout=stream, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            seg.exit_code = proc.wait()
            seg.state = "complete"
        except Interrupted:
            seg.state = "incomplete"
            raise
        finally:
            if proc.poll() is None or seg.state == "incomplete":
                # Kill the command's process group, including children of Bash.
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait()
    finally:
        stream.flush()
        os.fsync(stream.fileno())
        stream.seek(output_start)
        output = stream.read().decode("utf-8", errors="replace")
        if seg.is_filtered_cargo_test:
            parse_counts(output, seg)
        if log_dir:
            stream.seek(0, os.SEEK_END)
            stream.write(f"\n[state {seg.state}; exit {seg.exit_code}]\n".encode())
            os.fsync(stream.fileno())
        else:
            seg.output = output
        stream.close()


def _execute_criterion(*, root: Path, criterion: dict, report: dict,
                       log_dir: Path | None, env: dict) -> dict:
    """Shared execution engine. Production callers must use run_criterion."""
    criterion_id = criterion["id"]
    command = criterion["command"]
    segments_text = split_segments(command)
    if " && ".join(segments_text) != command or any(not s.strip() for s in segments_text):
        raise ValueError("segment reconstruction did not reproduce a nonempty recorded command")
    segments = [Segment(index=i, command=text) for i, text in enumerate(segments_text)]
    for seg in segments:
        cargo_test, seg.filter = classify(seg.command)
        seg.is_filtered_cargo_test = cargo_test and seg.filter is not None
        if log_dir:
            seg.log = str(log_dir / f"{criterion_id}-segment-{seg.index}.log")
    report.update({
        "criterion": criterion_id,
        "title": criterion.get("title", ""),
        "owners": criterion.get("owners", []),
        "command": command,
        "command_unmodified": report["contract"]["verified"],
        "status": "incomplete",
        "state": "incomplete",
        "exit_code": None,
        "started_at": now(),
        "insufficient_test_evidence_segments": [],
        "segments": [],
    })
    report_path = None

    def checkpoint() -> None:
        report["segments"] = [asdict(s) for s in segments]
        save_report(report_path, report)

    handlers = {}
    try:
        if log_dir:
            log_dir.mkdir(parents=True, exist_ok=True)
            report_path = log_dir / f"{criterion_id}-report.json"
            # Refuse reuse rather than destroying an earlier run's evidence.
            if report_path.exists() or any(s.log is not None and Path(s.log).exists() for s in segments):
                raise FileExistsError("evidence files already exist; use a fresh log directory")
            with report_path.open("x", encoding="utf-8") as stream:
                json.dump(report, stream)
                stream.flush()
                os.fsync(stream.fileno())
            report["report_path"] = str(report_path)
        for signum in (signal.SIGINT, signal.SIGTERM):
            handlers[signum] = signal.signal(signum, interrupt)
        checkpoint()
        for seg in segments:
            if report["exit_code"] not in (None, 0):
                seg.skipped = True
                seg.state = "skipped"
                continue
            seg.state = "incomplete"
            checkpoint()
            execute_segment(seg, root, env, log_dir)
            if seg.exit_code is None:
                raise OSError("command returned without an exit code; execution evidence is incomplete")
            if seg.exit_code != 0:
                report["exit_code"] = seg.exit_code if seg.exit_code > 0 else 128 - seg.exit_code
                report["status"] = "fail"
            elif seg.findings:
                report["insufficient_test_evidence_segments"].append(seg.index)
                report["exit_code"] = TEST_EVIDENCE_EXIT
                report["status"] = "insufficient-test-evidence"
            checkpoint()
        if report["exit_code"] is None:
            report["exit_code"] = 0
            report["status"] = "pass"
        # Production evidence is not complete until the post-run source and
        # contract checks in run_criterion have also finished.
        report["state"] = "incomplete" if report["contract"]["verified"] else "complete"
    except Interrupted as exc:
        report.update(status="interrupted", state="incomplete", exit_code=128 + exc.signum)
        for seg in segments:
            if seg.state == "pending":
                seg.skipped = True
                seg.state = "skipped"
    except OSError as exc:
        report.update(status="evidence-error", state="incomplete", exit_code=EVIDENCE_EXIT, error=str(exc))
        # Never overwrite a prior report when refusing reused evidence paths.
        if isinstance(exc, FileExistsError):
            report_path = None
    finally:
        for signum, handler in handlers.items():
            signal.signal(signum, handler)
    report["finished_at"] = now()
    checkpoint()
    return report


def run_criterion(*, root: Path, review: Path, criterion_id: str,
                  log_dir: Path | None, env: dict | None = None) -> dict:
    run_env = dict(os.environ)
    if env:
        run_env.update(env)
    report = {
        "criterion": criterion_id,
        "revision": None,
        "environment": environment_identity(root, run_env),
        "contract": {"reference": APPROVED_REFERENCE, "git_blob": APPROVED_BLOB, "verified": False},
        "command_unmodified": False,
        "segments": [],
    }
    try:
        source = source_identity(root)
        report.update(source)
        data, report["contract"] = approved_contract(root, review)
        criterion = next((c for c in data["criteria"] if c["id"].upper() == criterion_id.upper()), None)
        if criterion is None:
            raise KeyError(criterion_id)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        report.update(status="contract-error", state="not-started", exit_code=CONTRACT_EXIT, error=str(exc))
        return report
    report = _execute_criterion(root=root, criterion=criterion, report=report, log_dir=log_dir, env=run_env)
    commands_complete = report["status"] in ("pass", "fail", "insufficient-test-evidence")
    try:
        report["source_after"] = source_identity(root)
        if report["source_after"] != source:
            report.update(status="source-changed", exit_code=EVIDENCE_EXIT)
        approved_contract(root, review)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        report.update(status="contract-error", exit_code=CONTRACT_EXIT, error=str(exc))
    if commands_complete:
        report["state"] = "complete"
    if report.get("report_path"):
        save_report(Path(report["report_path"]), report)
    return report


def selftest() -> int:
    """Only this action loads the synthetic contract harness, never `run`."""
    suite = unittest.defaultTestLoader.discover(str(Path(__file__).parent), pattern="test_cutokyo_acceptance.py")
    if suite.countTestCases() == 0:
        print("selftest failed: runner test suite is missing or empty", file=sys.stderr)
        return 1
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["run", "selftest", "list"])
    parser.add_argument("--criterion", help="criterion id, e.g. C16")
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--review", help="path to the same pinned review.json")
    parser.add_argument("--log-dir", help="directory for streaming logs and a durable report; never reuse evidence files")
    parser.add_argument("--json", action="store_true", help="emit a JSON report, including diagnostics when no log directory is set")
    args = parser.parse_args()
    if args.action == "selftest":
        return selftest()
    root = Path(args.root).resolve()
    review = Path(args.review).resolve() if args.review else root / CONTRACT_PATH
    if args.action == "list":
        try:
            data, _ = approved_contract(root, review)
        except (ValueError, OSError, subprocess.CalledProcessError) as exc:
            print(f"acceptance contract rejected: {exc}", file=sys.stderr)
            return CONTRACT_EXIT
        for item in data["criteria"]:
            print(f"{item['id']}\t{item.get('title', '')}")
        return 0
    if not args.criterion:
        parser.error("run requires --criterion")
    try:
        report = run_criterion(
            root=root, review=review, criterion_id=args.criterion,
            log_dir=Path(args.log_dir).resolve() if args.log_dir else None,
        )
    except KeyError:
        print(f"unknown criterion: {args.criterion}", file=sys.stderr)
        return USAGE_EXIT
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"{report['criterion']}: {report['status']} (exit {report['exit_code']})")
        if report.get("error"):
            print(report["error"])
        for seg in report["segments"]:
            print(f"  [{seg['index']}] {seg['state']}, exit {seg['exit_code']}: {seg['command']}")
            for finding in seg["findings"]:
                print(f"      !! {finding}")
            if seg["log"]:
                print(f"      log: {seg['log']}")
            elif seg["output"]:
                print(seg["output"], end="")
    return report["exit_code"]


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Run a Cutokyo acceptance criterion exactly as recorded and refuse false greens.

The acceptance contract in review.json is read-only here. Each criterion command is
executed as its own `&&` segments so every segment can be attributed, and the tool
proves the rejoined segments are byte-identical to the recorded command before it
runs anything.

A filtered `cargo test` segment that selects no test exits 0 in Cargo. That is the
single failure mode that has repeatedly turned a missing test into a passing
criterion, so this runner treats it as failure ZERO_TEST_EXIT.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

ZERO_TEST_EXIT = 86
CONTRACT_EXIT = 87
USAGE_EXIT = 2

RUNNING = re.compile(r"^running (\d+) tests?$", re.MULTILINE)
RESULT = re.compile(
    r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out",
    re.MULTILINE,
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
    is_cargo_test: bool = False
    filter_name: str | None = None
    exit_code: int | None = None
    tests_run: int = 0
    tests_passed: int = 0
    tests_failed: int = 0
    targets: int = 0
    skipped: bool = False
    findings: list[str] = field(default_factory=list)

    def as_dict(self) -> dict:
        return {
            "index": self.index,
            "command": self.command,
            "is_filtered_cargo_test": self.is_cargo_test and self.filter_name is not None,
            "filter": self.filter_name,
            "exit_code": self.exit_code,
            "tests_run": self.tests_run,
            "tests_passed": self.tests_passed,
            "tests_failed": self.tests_failed,
            "test_targets": self.targets,
            "skipped": self.skipped,
            "findings": self.findings,
        }


def split_segments(command: str) -> list[str]:
    """Split on top-level ` && ` only. Quoted regions are preserved verbatim."""
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
    return [p for p in parts]


def classify(segment: str) -> tuple[bool, str | None]:
    """Return (is cargo test, explicit test-name filter)."""
    try:
        tokens = shlex.split(segment)
    except ValueError:
        return (False, None)
    if len(tokens) < 2 or tokens[0] != "cargo" or tokens[1] != "test":
        return (False, None)
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
        return (True, token)
    return (True, None)


def parse_counts(output: str) -> tuple[int, int, int, int]:
    targets = len(RUNNING.findall(output))
    run = sum(int(n) for n in RUNNING.findall(output))
    passed = sum(int(m.group(2)) for m in RESULT.finditer(output))
    failed = sum(int(m.group(3)) for m in RESULT.finditer(output))
    return run, passed, failed, targets


def load_criterion(review: Path, criterion_id: str) -> dict:
    data = json.loads(review.read_text(encoding="utf-8"))
    for item in data.get("criteria", []):
        if item.get("id", "").upper() == criterion_id.upper():
            return item
    raise KeyError(criterion_id)


def run_criterion(
    *,
    root: Path,
    review: Path,
    criterion_id: str,
    log_dir: Path | None,
    env: dict | None = None,
) -> dict:
    criterion = load_criterion(review, criterion_id)
    command = criterion["command"]

    segments_text = split_segments(command)
    rejoined = " && ".join(segments_text)
    if rejoined != command:
        return {
            "criterion": criterion_id,
            "status": "contract-error",
            "exit_code": CONTRACT_EXIT,
            "error": "segment reconstruction did not reproduce the recorded command",
            "command": command,
            "segments": [],
        }

    segments = [Segment(index=i, command=text) for i, text in enumerate(segments_text)]
    for seg in segments:
        seg.is_cargo_test, seg.filter_name = classify(seg.command)

    run_env = dict(os.environ)
    if env:
        run_env.update(env)

    overall = 0
    zero_test = []
    for seg in segments:
        if overall != 0:
            seg.skipped = True
            continue
        proc = subprocess.run(
            ["bash", "-c", seg.command],
            cwd=str(root),
            env=run_env,
            capture_output=True,
            text=True,
        )
        seg.exit_code = proc.returncode
        combined = proc.stdout + proc.stderr
        if log_dir:
            log_dir.mkdir(parents=True, exist_ok=True)
            (log_dir / f"{criterion_id}-segment-{seg.index}.log").write_text(
                f"$ {seg.command}\n\n{combined}\n[exit {proc.returncode}]\n",
                encoding="utf-8",
            )
        if seg.is_cargo_test:
            seg.tests_run, seg.tests_passed, seg.tests_failed, seg.targets = parse_counts(combined)
            if seg.filter_name and proc.returncode == 0 and seg.tests_run == 0:
                seg.findings.append(
                    f"filter {seg.filter_name!r} selected no test; Cargo still exited 0"
                )
                zero_test.append(seg.index)
        if proc.returncode != 0:
            overall = proc.returncode

    status = "pass"
    exit_code = overall
    if overall != 0:
        status = "fail"
    elif zero_test:
        status = "zero-test"
        exit_code = ZERO_TEST_EXIT

    return {
        "criterion": criterion_id,
        "title": criterion.get("title", ""),
        "owners": criterion.get("owners", []),
        "status": status,
        "exit_code": exit_code,
        "command": command,
        "command_unmodified": True,
        "zero_test_segments": zero_test,
        "segments": [s.as_dict() for s in segments],
    }


def selftest() -> int:
    """Known-good passes, known-bad fails, and errors can never become success."""
    failures: list[str] = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        if ok:
            print(f"[PASS] {name}")
        else:
            failures.append(f"{name}: {detail}")
            print(f"[FAIL] {name} {detail}")

    # Splitting must preserve quoted text and never lose a segment.
    cmd = "a --x && b 'q && q' && c"
    parts = split_segments(cmd)
    check("split preserves quoted &&", parts == ["a --x", "b 'q && q'", "c"], str(parts))
    check("split round-trips", " && ".join(parts) == cmd)

    # Filter classification.
    check(
        "detects filtered cargo test",
        classify("cargo test -p cutokyo-integration-tests --test e2e some_name") == (True, "some_name"),
    )
    check(
        "unfiltered cargo test is not a filter",
        classify("cargo test --workspace") == (True, None),
    )
    check(
        "non-cargo segment ignored",
        classify("python3 tools/fleet/cutokyo-gates.py legal --root .") == (False, None),
    )
    check(
        "flag values are not filters",
        classify("cargo test -p a --test b --features c") == (True, None),
    )

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        shim = root / "bin"
        shim.mkdir()
        # A cargo shim whose behavior is chosen by the filter name.
        (shim / "cargo").write_text(
            "#!/bin/bash\n"
            'for a in "$@"; do\n'
            '  case "$a" in\n'
            "    good) echo 'running 1 test'; echo 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out'; exit 0;;\n"
            "    empty) echo 'running 0 tests'; echo 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out'; exit 0;;\n"
            "    broken) echo 'running 1 test'; echo 'test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out'; exit 101;;\n"
            "  esac\n"
            "done\n"
            "echo 'running 3 tests'; echo 'test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'; exit 0\n",
            encoding="utf-8",
        )
        (shim / "cargo").chmod(0o755)
        env = {"PATH": f"{shim}:{os.environ['PATH']}"}

        def write_review(command: str) -> Path:
            path = root / "review.json"
            path.write_text(
                json.dumps({"criteria": [{"id": "CXX", "title": "t", "owners": [], "command": command}]}),
                encoding="utf-8",
            )
            return path

        good = run_criterion(
            root=root, review=write_review("cargo test -p x --test e2e good"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check("known-good exits 0", good["exit_code"] == 0 and good["status"] == "pass", json.dumps(good["segments"]))

        empty = run_criterion(
            root=root, review=write_review("cargo test -p x --test e2e empty"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check(
            "zero-test filter cannot pass",
            empty["exit_code"] == ZERO_TEST_EXIT and empty["status"] == "zero-test",
            json.dumps(empty["segments"]),
        )

        broken = run_criterion(
            root=root, review=write_review("cargo test -p x --test e2e broken"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check("failing test propagates", broken["exit_code"] == 101 and broken["status"] == "fail")

        mixed = run_criterion(
            root=root,
            review=write_review("cargo test -p x --test e2e good && cargo test -p x --test e2e empty"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check(
            "later zero-test segment still fails the criterion",
            mixed["exit_code"] == ZERO_TEST_EXIT and mixed["zero_test_segments"] == [1],
        )

        missing = run_criterion(
            root=root, review=write_review("cutokyo-does-not-exist --json"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check("missing executable is not success", missing["exit_code"] == 127 and missing["status"] == "fail")

        unfiltered_zero = run_criterion(
            root=root, review=write_review("cargo test -p x --lib"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check("unfiltered suite is judged by cargo alone", unfiltered_zero["exit_code"] == 0)

        stop = run_criterion(
            root=root,
            review=write_review("cargo test -p x --test e2e broken && cargo test -p x --test e2e good"),
            criterion_id="CXX", log_dir=None, env=env,
        )
        check(
            "segments stop at first failure like &&",
            stop["exit_code"] == 101 and stop["segments"][1]["skipped"] is True,
        )

    if failures:
        print(f"\nselftest FAILED: {len(failures)} case(s)")
        return 1
    print("\nselftest OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["run", "selftest", "list"])
    parser.add_argument("--criterion", help="criterion id, e.g. C16")
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--review", help="path to review.json")
    parser.add_argument("--log-dir", help="directory for per-segment logs")
    parser.add_argument("--json", action="store_true", help="emit a JSON report")
    args = parser.parse_args()

    if args.action == "selftest":
        return selftest()

    root = Path(args.root).resolve()
    review = Path(args.review).resolve() if args.review else (
        root / ".claude/fleets/20260919-cutokyo-v01/review.json"
    )
    if not review.is_file():
        print(f"acceptance contract not found: {review}", file=sys.stderr)
        return USAGE_EXIT

    if args.action == "list":
        data = json.loads(review.read_text(encoding="utf-8"))
        for item in data.get("criteria", []):
            print(f"{item['id']}\t{item.get('title','')}")
        return 0

    if not args.criterion:
        print("run requires --criterion", file=sys.stderr)
        return USAGE_EXIT

    try:
        report = run_criterion(
            root=root,
            review=review,
            criterion_id=args.criterion,
            log_dir=Path(args.log_dir).resolve() if args.log_dir else None,
        )
    except KeyError:
        print(f"unknown criterion: {args.criterion}", file=sys.stderr)
        return USAGE_EXIT

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"{report['criterion']}: {report['status']} (exit {report['exit_code']})")
        for seg in report["segments"]:
            state = "skipped" if seg["skipped"] else f"exit {seg['exit_code']}"
            extra = ""
            if seg["is_filtered_cargo_test"]:
                extra = f" [{seg['tests_run']} run, {seg['tests_passed']} passed]"
            print(f"  [{seg['index']}] {state}{extra}: {seg['command']}")
            for finding in seg["findings"]:
                print(f"      !! {finding}")
    return report["exit_code"]


if __name__ == "__main__":
    sys.exit(main())

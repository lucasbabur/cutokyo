"""Runner-only fixtures. Synthetic contracts never enter the production CLI.

The selftest action loads this module; run/list cannot opt out of the pinned
contract. The subprocess entry point below exists solely to test interruption of
synthetic commands without touching the approved review.json.
"""

from __future__ import annotations

import importlib.util
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

RUNNER = Path(__file__).with_name("cutokyo-acceptance.py")
spec = importlib.util.spec_from_file_location("cutokyo_acceptance", RUNNER)
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)


def synthetic(root: Path, command: str, log_dir: Path | None = None, env: dict | None = None) -> dict:
    """Exercise execution only. Reports explicitly cannot claim approved evidence."""
    return runner._execute_criterion(
        root=root,
        criterion={"id": "CXX", "command": command},
        report={"synthetic_test_harness": True, "contract": {"verified": False}},
        log_dir=log_dir,
        env=env or dict(os.environ),
    )


class AcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cutokyo-acceptance-selftest-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def parse(self, output):
        seg = runner.Segment(index=0, command="cargo test chosen")
        runner.parse_counts(output, seg)
        return seg

    def test_split_and_classification(self):
        command = "a --x && b 'q && q' && c"
        self.assertEqual(runner.split_segments(command), ["a --x", "b 'q && q'", "c"])
        self.assertEqual(" && ".join(runner.split_segments(command)), command)
        self.assertEqual(runner.classify("cargo test -p a --test b chosen"), (True, "chosen"))
        self.assertEqual(runner.classify("cargo test -p a --test b --features c"), (True, None))
        self.assertEqual(runner.classify("cargo test --workspace"), (True, None))
        self.assertEqual(runner.classify("python3 some.py"), (False, None))

    def test_consistent_multiple_targets(self):
        seg = self.parse(
            "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.00s\n"
            "running 2 tests\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 1 filtered out; finished in 0.00s\n"
        )
        self.assertEqual(seg.findings, [])
        self.assertEqual((seg.tests_passed, seg.tests_ignored, seg.test_targets, seg.test_results), (1, 1, 2, 2))

    def test_missing_inconsistent_and_failed_results(self):
        outputs = {
            "missing result": "running 1 test\n",
            "no counts": "compiler completed\n",
            "result without selection": "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "count mismatch": "running 2 tests\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "wrong status": "running 1 test\ntest result: FAILED. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "failed": "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "malformed result": "running 1 test\ntest result: ok. unexpected\n",
            # Aggregate totals can agree even though individual targets disagree.
            "cross target mismatch": "running 2 tests\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\nrunning 0 tests\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "second result missing": "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\nrunning 0 tests\n",
        }
        for name, output in outputs.items():
            with self.subTest(name=name):
                self.assertTrue(self.parse(output).findings)

    def shim_env(self, output: str, exit_code: int = 0) -> dict:
        shim = self.root / "bin"
        shim.mkdir(exist_ok=True)
        cargo = shim / "cargo"
        cargo.write_text(f"#!{sys.executable}\nimport sys\nsys.stdout.write({output!r})\nsys.exit({exit_code})\n")
        cargo.chmod(0o755)
        return {**os.environ, "PATH": f"{shim}:{os.environ['PATH']}"}

    def test_shim_false_greens_fail_and_stop_later_segments(self):
        for name, output in {
            "ignored only": "running 1 test\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n",
            "zero selected": "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out\n",
            "missing result": "running 1 test\n",
            "lying exit status": "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
        }.items():
            with self.subTest(name=name):
                report = synthetic(self.root, "cargo test chosen && touch should-not-run", env=self.shim_env(output))
                self.assertEqual(report["exit_code"], 86)
                self.assertEqual(report["status"], "insufficient-test-evidence")
                self.assertEqual(report["insufficient_test_evidence_segments"], [0])
                self.assertTrue(report["segments"][1]["skipped"])
                self.assertFalse((self.root / "should-not-run").exists())
                self.assertIn(output, report["segments"][0]["output"])
                self.assertFalse(report["command_unmodified"])

    def test_later_empty_target_fails(self):
        env = self.shim_env("running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out\n")
        report = synthetic(self.root, "true && cargo test chosen", env=env)
        self.assertEqual(report["exit_code"], 86)
        self.assertEqual(report["insufficient_test_evidence_segments"], [1])

    def test_process_failures_propagate_and_keep_both_streams(self):
        report = synthetic(self.root, "printf 'stdout diagnostic'; printf 'stderr diagnostic' >&2; exit 23 && touch later")
        self.assertEqual(report["exit_code"], 23)
        self.assertEqual(report["status"], "fail")
        self.assertIn("stdout diagnostic", report["segments"][0]["output"])
        self.assertIn("stderr diagnostic", report["segments"][0]["output"])
        self.assertTrue(report["segments"][1]["skipped"])
        missing = synthetic(self.root, "cutokyo-command-that-does-not-exist")
        self.assertEqual(missing["exit_code"], 127)
        self.assertIn("not found", missing["segments"][0]["output"])

    def test_unfiltered_cargo_is_judged_by_exit_status(self):
        report = synthetic(self.root, "cargo test --workspace", env=self.shim_env("running 0 tests\n"))
        self.assertEqual(report["exit_code"], 0)

    def test_log_contents_and_refuse_overwrite(self):
        logs = self.root / "logs"
        report = synthetic(self.root, "printf 'a'; printf 'b' >&2", logs)
        self.assertEqual(report["state"], "complete")
        self.assertEqual(report["exit_code"], 0)
        path = logs / "CXX-segment-0.log"
        before = path.read_bytes()
        report_before = (logs / "CXX-report.json").read_bytes()
        self.assertIn(b"ab\n[state complete; exit 0]", before)
        self.assertEqual(json.loads(report_before), report)
        second = synthetic(self.root, "printf 'replacement'", logs)
        self.assertEqual(second["exit_code"], 88)
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual((logs / "CXX-report.json").read_bytes(), report_before)

    def wait_for_output(self, proc, log):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if log.exists() and b"\npartial stderr\n" in log.read_bytes():
                return
            if proc.poll() is not None:
                self.fail(f"fixture exited early: {proc.communicate()}")
            time.sleep(0.02)
        self.fail("timed out waiting for live output")

    def test_interruptions_keep_live_partial_logs_and_incomplete_report(self):
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGKILL):
            with self.subTest(signal=signum):
                logs = self.root / f"logs-{signum}"
                proc = subprocess.Popen(
                    [sys.executable, str(Path(__file__)), "--fixture", str(self.root), str(logs)],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                )
                child_pid = None
                try:
                    self.wait_for_output(proc, logs / "CXX-segment-0.log")
                    child_pid = int((logs / "child.pid").read_text())
                    live = json.loads((logs / "CXX-report.json").read_text())
                    self.assertEqual(live["state"], "incomplete")
                    self.assertIsNone(live["exit_code"])
                    proc.send_signal(signum)
                    stdout, stderr = proc.communicate(timeout=10)
                    self.assertEqual(stderr, "")
                    disk = json.loads((logs / "CXX-report.json").read_text())
                    self.assertEqual(disk["state"], "incomplete")
                    self.assertNotEqual(disk["status"], "pass")
                    if signum != signal.SIGKILL:
                        report = json.loads(stdout)
                        self.assertEqual(report, disk)
                        self.assertEqual(report["exit_code"], 128 + signum)
                        self.assertEqual(report["status"], "interrupted")
                        self.assertTrue(report["segments"][1]["skipped"])
                    partial = (logs / "CXX-segment-0.log").read_text()
                    self.assertIn("partial stdout", partial)
                    self.assertIn("partial stderr", partial)
                    self.assertFalse((logs / "later").exists())
                finally:
                    if proc.poll() is None:
                        proc.kill()
                        proc.communicate()
                    # SIGKILL cannot run the runner's process-group cleanup.
                    if child_pid:
                        try:
                            os.killpg(child_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass

    def test_pinned_contract_checks_both_selected_tree_and_override(self):
        repo = RUNNER.parents[2]
        data, identity = runner.approved_contract(repo, repo / runner.CONTRACT_PATH)
        self.assertTrue(identity["verified"])
        self.assertEqual(identity["git_blob"], runner.APPROVED_BLOB)
        approved = runner.git(repo, "show", runner.APPROVED_REFERENCE)
        self.assertEqual(json.loads(approved), data)
        # A disposable Git object database contains the approved blob/reference,
        # while its checked-out review is mutated. No real contract is edited.
        subprocess.run(["git", "clone", "--shared", "--no-checkout", str(repo), str(self.root / "repo")], check=True, capture_output=True)
        selected = self.root / "repo"
        subprocess.run(["git", "-C", str(selected), "reset", "--hard", runner.APPROVED_REVISION], check=True, capture_output=True)
        review = selected / runner.CONTRACT_PATH
        command = next(c["command"] for c in data["criteria"] if c["id"] == "C03")
        env = self.shim_env("running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n")
        # Exercise production validation with mocked executables, not a changed
        # contract. This is a runner test, never product acceptance evidence.
        accepted = runner.run_criterion(root=selected, review=review, criterion_id="C03", log_dir=None, env=env)
        self.assertEqual(accepted["exit_code"], 0)
        self.assertEqual(accepted["state"], "complete")
        self.assertTrue(accepted["command_unmodified"])
        self.assertEqual(accepted["command"], command)
        self.assertEqual(" && ".join(s["command"] for s in accepted["segments"]), command)
        self.assertEqual(accepted["source_after"]["revision"], accepted["revision"])
        self.assertEqual(len(accepted["contract"]["sha256"]), 64)
        self.assertTrue(all(s["output"] for s in accepted["segments"]))
        cli = subprocess.run(
            [sys.executable, str(RUNNER), "run", "--root", str(selected), "--criterion", "C03", "--json"],
            env={**env, "SYNTHETIC_CREDENTIAL": "not-a-real-secret-never-dump-env"},
            capture_output=True, text=True, check=True,
        )
        self.assertIn("test result: ok", json.loads(cli.stdout)["segments"][0]["output"])
        self.assertNotIn("not-a-real-secret-never-dump-env", cli.stdout)
        # Mutate an already-dirty tracked file: status filenames alone would not
        # detect this; the source fingerprint must change as well.
        readme = selected / "README.md"
        original_readme = readme.read_bytes()
        readme.write_bytes(original_readme + b"\nselftest before\n")
        cargo = self.root / "bin/cargo"
        cargo.write_text(cargo.read_text().replace("import sys", "import sys\nfrom pathlib import Path\np=Path('README.md')\np.write_bytes(p.read_bytes()+b'changed')"))
        changed = runner.run_criterion(root=selected, review=review, criterion_id="C03", log_dir=None, env=env)
        self.assertEqual(changed["exit_code"], 88)
        self.assertEqual(changed["status"], "source-changed")
        self.assertNotEqual(changed["tracked_diff_sha256"], changed["source_after"]["tracked_diff_sha256"])
        readme.write_bytes(original_readme)
        alternate = self.root / "approved.json"
        alternate.write_bytes(approved)
        review.write_bytes(approved + b"\n")
        report = runner.run_criterion(root=selected, review=alternate, criterion_id="C03", log_dir=None)
        self.assertEqual(report["exit_code"], 87)
        self.assertEqual(report["segments"], [])
        self.assertFalse(report["command_unmodified"])
        self.assertEqual(report["revision"], runner.APPROVED_REVISION)
        self.assertTrue(report["environment"]["id"])
        review.write_bytes(approved)
        alternate.write_text('{"criteria": [{"id": "C03", "command": "true"}]}')
        report = runner.run_criterion(root=selected, review=alternate, criterion_id="C03", log_dir=None)
        self.assertEqual(report["exit_code"], 87)
        self.assertEqual(report["segments"], [])
        # Moving a local main reference cannot redefine the approval anchor.
        subprocess.run(["git", "-C", str(selected), "update-ref", "refs/heads/main", "HEAD~1"], check=True)
        runner.approved_contract(selected, review)

    def test_production_cli_rejects_synthetic_contract(self):
        review = self.root / "review.json"
        review.write_text('{"criteria": [{"id": "CXX", "command": "touch executed"}]}')
        proc = subprocess.run(
            [sys.executable, str(RUNNER), "run", "--root", str(self.root), "--review", str(review), "--criterion", "CXX", "--json"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 87)
        self.assertEqual(json.loads(proc.stdout)["status"], "contract-error")
        self.assertFalse((self.root / "executed").exists())

    def test_real_rust_passing_failing_ignored_and_zero_selected(self):
        (self.root / "Cargo.toml").write_text(
            '[package]\nname = "acceptance-evidence-fixture"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n'
        )
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text(
            '#[test]\nfn passing() { assert_eq!(2 + 2, 4); }\n'
            '#[test]\n#[ignore]\nfn ignored_only() { panic!("must not execute"); }\n'
            '#[test]\nfn failing() { panic!("deliberate fixture failure"); }\n'
        )
        # Resolve the installed compiler before isolating Cargo config. Point at
        # its real bin directory so version-manager shims cannot install tools in
        # the disposable CARGO_HOME or turn fixture execution into a download.
        sysroot = subprocess.run(["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True).stdout.strip()
        env = {
            **os.environ,
            "PATH": f"{Path(sysroot) / 'bin'}:{os.environ['PATH']}",
            "CARGO_HOME": str(self.root / "cargo-home"),
            "CARGO_TARGET_DIR": str(self.root / "target"),
        }
        for name, expected in (("passing", 0), ("ignored_only", 86), ("failing", 101), ("zero_selected", 86)):
            with self.subTest(filter=name):
                command = f"cargo test --offline --lib {name}"
                report = synthetic(self.root, command, env=env)
                self.assertEqual(report["exit_code"], expected, json.dumps(report, indent=2))
                self.assertEqual(report["command"], command)
                self.assertEqual(report["segments"][0]["command"], command)
                print(f"REAL RUST: {command} -> {report['status']} exit {report['exit_code']}")
                print(report["segments"][0]["output"])


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--fixture":
        root, logs = map(Path, sys.argv[2:])
        logs.mkdir()
        child = (
            "import os,sys,time; from pathlib import Path; "
            f"Path({str(logs / 'child.pid')!r}).write_text(str(os.getpid())); "
            "print('partial stdout', flush=True); print('partial stderr', file=sys.stderr, flush=True); time.sleep(60)"
        )
        command = f"{runner.shlex.quote(sys.executable)} -c {runner.shlex.quote(child)} && touch {runner.shlex.quote(str(logs / 'later'))}"
        print(json.dumps(synthetic(root, command, logs)))
    else:
        unittest.main(verbosity=2)

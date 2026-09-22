#!/usr/bin/env python3
"""Fast mutation tests for release validators; never artifact acceptance evidence."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def module(name: str, relative: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    assert spec is not None and spec.loader is not None
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


manifest = module("release_manifest", "tools/release/create-manifest.py")
snapshot = module("source_snapshot", "tools/scripts/source-snapshot.py")


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout"
        self.root.mkdir()
        self.source = self.root / "source.txt"
        self.source.write_text("original\n")
        self.git("init", "-q")
        self.git("config", "user.name", "Release fixture")
        self.git("config", "user.email", "fixture@cutokyo.invalid")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.sequence = 0

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.root), *args], check=True,
                              capture_output=True, text=True).stdout.strip()

    def invoke(self, *args):
        return subprocess.run([sys.executable, str(ROOT / "tools/scripts/source-snapshot.py"),
                               "--root", str(self.root), *map(str, args)],
                              capture_output=True, text=True, timeout=15)

    def external(self):
        self.sequence += 1
        return Path(self.temporary.name) / f"evidence-{self.sequence}.json"

    def capture(self):
        path = self.external()
        result = self.invoke("--capture", path)
        self.assertEqual(result.returncode, 0, result.stderr)
        return path

    def test_clean_capture_and_malformed_fingerprints(self):
        path = self.capture()
        good = self.invoke("--verify", path)
        self.assertEqual(good.returncode, 0, good.stderr)
        value = json.loads(path.read_text())
        for invalid in (None, [], {"source.txt": [1]}, {"source.txt": "not-a-fingerprint"}):
            value["fingerprints"] = invalid
            path.write_text(json.dumps(value))
            bad = self.invoke("--verify", path)
            self.assertEqual(bad.returncode, 86, bad.stderr)
            self.assertIn("snapshot fingerprints are invalid", bad.stderr)
            self.assertNotIn("Traceback", bad.stderr)

    def test_split_capture_detects_restored_source_and_mtime(self):
        path = self.capture()
        metadata = self.source.stat()
        self.source.write_text("temporary")
        self.source.write_text("original\n")
        os.utime(self.source, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
        result = self.invoke("--verify", path)
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertIn("source.txt", json.loads(result.stderr)["changed_paths"])

    def test_split_capture_detects_head_round_trip(self):
        path = self.capture()
        original = self.git("rev-parse", "HEAD")
        self.git("commit", "--allow-empty", "-qm", "temporary head")
        self.git("reset", "--soft", original)
        result = self.invoke("--verify", path)
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertTrue(json.loads(result.stderr)["state_changes"])

    def test_split_capture_detects_index_round_trip(self):
        path = self.capture()
        self.git("update-index", "--assume-unchanged", "source.txt")
        self.git("update-index", "--no-assume-unchanged", "source.txt")
        result = self.invoke("--verify", path)
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertIn("index", json.loads(result.stderr)["state_changes"])

    def test_evidence_refuses_replacement_before_command(self):
        path = self.external()
        path.write_text("preserve me")
        result = self.invoke("--manifest", path, "--", sys.executable, "-c", "raise SystemExit(99)")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(path.read_text(), "preserve me")

    def test_interrupted_child_leaves_a_failed_source_receipt(self):
        result = self.invoke("--", sys.executable, "-c",
                             "from pathlib import Path; Path('source.txt').write_text('interrupted'); "
                             "raise SystemExit(130)")
        self.assertEqual(result.returncode, 86, result.stderr)
        receipt = json.loads(result.stderr)
        self.assertEqual(receipt["command_exit_code"], 130)
        self.assertEqual(receipt["changed_paths"], ["source.txt"])

    def test_evidence_cannot_point_into_git_metadata(self):
        result = self.invoke("--capture", self.root / ".git" / "capture.json")
        self.assertEqual(result.returncode, 2)
        self.assertFalse((self.root / ".git" / "capture.json").exists())

    def test_command_detects_fast_write_restore(self):
        result = self.invoke("--", sys.executable, "-c",
                             "from pathlib import Path; import os; p=Path('source.txt'); "
                             "s=p.stat(); b=p.read_bytes(); p.write_bytes(b'changed'); "
                             "p.write_bytes(b); os.utime(p,ns=(s.st_atime_ns,s.st_mtime_ns))")
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertEqual(json.loads(result.stderr)["temporary_changed_paths"], ["source.txt"])


class SbomTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / "fixture.cdx.json"
        self.value = {
            "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "metadata": {"component": {"type": "application", "name": "fixture",
                                       "version": "1.0.0", "bom-ref": "app"}},
            "components": [{"type": "library", "name": "dependency", "version": "1.0.0", "bom-ref": "lib"}],
            "dependencies": [{"ref": "app", "dependsOn": ["lib"]}],
        }

    def validate(self):
        self.path.write_text(json.dumps(self.value))
        manifest.validate_sboms([self.path], self.root)

    def test_good(self):
        self.validate()

    def test_null_product_fails_cleanly(self):
        self.value["metadata"]["component"] = None
        with self.assertRaises(SystemExit) as failure:
            self.validate()
        self.assertEqual(failure.exception.code, 1)

    def test_dependency_shapes_fail_cleanly(self):
        for edge in ({"ref": [], "dependsOn": []}, {"ref": "app", "dependsOn": [{}]}, None):
            with self.subTest(edge=edge):
                self.value["dependencies"] = [edge]
                with self.assertRaises(SystemExit) as failure:
                    self.validate()
                self.assertEqual(failure.exception.code, 1)


if __name__ == "__main__":
    unittest.main()

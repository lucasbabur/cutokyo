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

    def test_split_api_is_not_supported(self):
        for option in ("--capture", "--verify"):
            result = self.invoke(option, self.external())
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("unrecognized arguments", result.stderr)

    def test_command_detects_head_round_trip(self):
        result = self.invoke("--", sys.executable, "-c",
                             "import subprocess; "
                             "head=subprocess.check_output(['git','rev-parse','HEAD']).strip(); "
                             "subprocess.run(['git','commit','--allow-empty','-qm','temporary'],check=True); "
                             "subprocess.run(['git','reset','--soft',head],check=True)")
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertTrue(json.loads(result.stderr)["state_changes"])

    def test_command_detects_index_round_trip(self):
        result = self.invoke("--", sys.executable, "-c",
                             "import subprocess; "
                             "subprocess.run(['git','update-index','--assume-unchanged','source.txt'],check=True); "
                             "subprocess.run(['git','update-index','--no-assume-unchanged','source.txt'],check=True)")
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertIn("index", json.loads(result.stderr)["state_changes"])

    def test_direct_ref_round_trip_and_packed_override(self):
        original = self.git("rev-parse", "HEAD")
        self.git("commit", "--allow-empty", "-qm", "alternate")
        alternate = self.git("rev-parse", "HEAD")
        self.git("reset", "--soft", original)
        reference = self.root / ".git" / self.git("symbolic-ref", "HEAD")
        for packed in (False, True):
            if packed:
                self.git("pack-refs", "--all", "--prune")
            code = (
                "from pathlib import Path; import subprocess,sys,os; p=Path(sys.argv[1]); "
                "b=p.read_bytes() if p.exists() else None; "
                "p.write_text(sys.argv[2]+'\\n'); "
                "assert subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()==sys.argv[2]; "
                "p.write_bytes(b) if b is not None else p.unlink()"
            )
            result = self.invoke("--", sys.executable, "-c", code, reference, alternate)
            self.assertEqual(result.returncode, 86, result.stderr)
            receipt = json.loads(result.stderr)
            self.assertEqual(receipt["command_exit_code"], 0)
            self.assertTrue(receipt["state_changes"])
            self.assertEqual(self.git("rev-parse", "HEAD"), original)

    def test_packed_ref_write_restore(self):
        self.git("pack-refs", "--all", "--prune")
        result = self.invoke("--", sys.executable, "-c",
                             "from pathlib import Path; p=Path('.git/packed-refs'); "
                             "b=p.read_bytes(); p.write_bytes(b+b'# temporary\\n'); p.write_bytes(b)")
        self.assertEqual(result.returncode, 86, result.stderr)
        self.assertIn("packed-refs", json.loads(result.stderr)["state_changes"])

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
        result = self.invoke("--manifest", self.root / ".git" / "capture.json", "--", sys.executable, "-c", "pass")
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

    def test_leaf_dependencies_may_omit_optional_depends_on(self):
        self.value["dependencies"].append({"ref": "lib"})
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

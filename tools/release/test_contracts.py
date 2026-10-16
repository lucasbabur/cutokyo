#!/usr/bin/env python3
"""Fast mutation tests for release validators; never artifact acceptance evidence."""
from __future__ import annotations

import io
import tarfile
from unittest.mock import patch
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


publisher = module("cli_publisher", "tools/release/publish-cli-assets.py")

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

    def test_transiently_unresolvable_head_is_recorded_not_fatal(self):
        self.git("pack-refs", "--all", "--prune")
        result = self.invoke("--", sys.executable, "-c",
                             "from pathlib import Path; import time; p=Path('.git/packed-refs'); "
                             "b=p.read_bytes(); p.write_bytes(b''); time.sleep(0.5); p.write_bytes(b)")
        self.assertEqual(result.returncode, 86, result.stderr)
        receipt = json.loads(result.stderr)
        self.assertEqual(receipt["command_exit_code"], 0)
        self.assertIn("HEAD", receipt["state_changes"])

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


class CliPublicationTests(unittest.TestCase):
    """Synthetic release bytes; never a claim of native installer acceptance."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tag = "v1.2.3"
        self.repo = "lucasbabur/cutokyo"
        self.remote = {}
        self.uploaded = []
        self.draft = False
        self.metadata = {
            "name": "cutokyo", "version": "1.2.3",
            "repository": f"https://github.com/{self.repo}",
            "artifactDownloadUrls": [f"https://github.com/{self.repo}/releases/download/{self.tag}"],
            "supportedPlatforms": {},
        }
        for target, extension in publisher.TARGETS.items():
            name = f"cutokyo-cli-{target}{extension}"
            native = self.root / f"cli-{target}" / name
            native.parent.mkdir()
            native.write_bytes(f"SYNTHETIC-{target}".encode())
            native.with_name(name + ".sha256").write_text(publisher.digest(native) + " *" + name)
            self.metadata["supportedPlatforms"][target] = {"artifactName": name}
        (self.root / "global-installers").mkdir()
        self.pack()

    def pack(self):
        content = json.dumps(self.metadata).encode()
        with tarfile.open(self.root / "global-installers/cutokyo-1.2.3.tgz", "w:gz") as archive:
            info = tarfile.TarInfo("package/package.json")
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))

    def gh(self, *args):
        action = args[1]
        if action == "view":
            return json.dumps({"isDraft": self.draft, "tagName": self.tag,
                               "assets": [{"name": name} for name in self.remote]})
        if action == "download":
            name = args[args.index("--pattern") + 1]
            directory = args[args.index("--dir") + 1]
            (Path(directory) / name).write_bytes(self.remote[name])
        elif action == "upload":
            self.assertNotIn("--clobber", args)
            for value in args[5:]:
                path = Path(value)
                self.assertNotIn(path.name, self.remote)
                self.remote[path.name] = path.read_bytes()
                self.uploaded.append(path.name)
        else:
            self.fail(f"unexpected gh action: {action}")
        return ""

    def publish(self, public=None):
        if public is None:
            public = lambda url, **kwargs: io.BytesIO(self.remote[url.rsplit("/", 1)[1]])
        with patch.object(publisher, "gh", side_effect=self.gh), \
                patch.object(publisher.urllib.request, "urlopen", side_effect=public):
            publisher.publish(self.root, self.tag, self.repo)

    def test_publish_all_four_targets_and_retry_identical_bytes(self):
        self.publish()
        self.assertEqual(len(self.uploaded), 9)
        self.uploaded.clear()
        self.publish()
        self.assertEqual(self.uploaded, [])

    def test_changed_existing_asset_refused_before_any_upload(self):
        name = "cutokyo-cli-x86_64-pc-windows-msvc.zip"
        self.remote[name] = b"DIFFERENT-SYNTHETIC-BYTES"
        with self.assertRaisesRegex(ValueError, "refusing to replace"):
            self.publish()
        self.assertEqual(self.uploaded, [])

    def test_missing_native_platform_refused(self):
        next(self.root.glob("cli-*/*.zip")).unlink()
        with self.assertRaises(FileNotFoundError):
            self.publish()
        self.assertEqual(self.uploaded, [])

    def test_corrupt_archive_refused(self):
        next(self.root.glob("cli-*/*.zip")).write_bytes(b"CORRUPT")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.publish()
        self.assertEqual(self.uploaded, [])

    def test_wrong_npm_identity_and_downloads_refused(self):
        for key in ("name", "version", "repository", "artifactDownloadUrls"):
            original = self.metadata[key]
            self.metadata[key] = "wrong"
            self.pack()
            with self.assertRaisesRegex(ValueError, "do not match"):
                self.publish()
            self.metadata[key] = original
        self.assertEqual(self.uploaded, [])

    def test_draft_release_refused(self):
        self.draft = True
        with self.assertRaisesRegex(ValueError, "public release"):
            self.publish()
        self.assertEqual(self.uploaded, [])

    def test_anonymous_download_must_match_tested_bytes(self):
        with self.assertRaisesRegex(ValueError, "public download differs"):
            self.publish(public=lambda *args, **kwargs: io.BytesIO(b"WRONG-PUBLIC-BYTES"))

    def test_fork_and_nonstable_tag_refused(self):
        for tag, repo in ((self.tag, "example/fork"), ("v1.2.3-rc.1", self.repo)):
            with self.assertRaisesRegex(ValueError, "only stable tags"):
                publisher.candidates(self.root, tag, repo)


if __name__ == "__main__":
    unittest.main()

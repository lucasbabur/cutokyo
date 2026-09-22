"""Non-browser regression tests for package provenance and process isolation."""

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("native_e2e", Path(__file__).with_name("native-e2e.py"))
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Native package runner module could not be loaded")
NATIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NATIVE)


class NativePackageHarness(unittest.TestCase):
    def test_concurrent_roots_ports_and_runtime_home_are_isolated(self):
        with tempfile.TemporaryDirectory(prefix="cutokyo-native-test-wdio-") as first, \
                tempfile.TemporaryDirectory(prefix="cutokyo-native-test-wdio-") as second, \
                NATIVE.reserve_port() as first_port, NATIVE.reserve_port() as second_port:
            roots = [Path(first), Path(second)]
            ports = [first_port.getsockname()[1], second_port.getsockname()[1]]
            self.assertNotEqual(ports[0], ports[1])
            self.assertNotEqual(roots[0], roots[1])
            for root, port in zip(roots, ports):
                env = NATIVE.build_environment(root)
                self.assertEqual(env["CARGO_TARGET_DIR"], str(root / "target"))
                self.assertEqual(env["CUTOKYO_NATIVE_ASSET_DIR"], str(root / "assets"))
                self.assertEqual(NATIVE.tauri_config(root)["build"]["frontendDist"], str(root / "assets"))
                runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", port)
                self.assertEqual(runtime["CUTOKYO_NATIVE_PORT"], str(port))
                self.assertEqual(runtime["CUTOKYO_DESKTOP_TEST_MODE"], "1")
                self.assertEqual(runtime["CUTOKYO_DESKTOP_TEST_ROOT"], str(root))
                for key in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"]:
                    self.assertTrue(Path(runtime[key]).is_relative_to(root))
                    self.assertEqual(Path(runtime[key]).stat().st_mode & 0o777, 0o700)
                self.assertEqual(runtime["PATH"].split(os.pathsep)[0], str(root / "bin"))

    def test_runtime_environment_excludes_credentials_and_loader_hooks(self):
        excluded = {
            "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "NPM_TOKEN", "AWS_SECRET_ACCESS_KEY",
            "CLAUDE_CONFIG_DIR", "CODEX_HOME", "NODE_OPTIONS", "LD_PRELOAD", "HTTPS_PROXY",
            "CUTOKYO_CONFIG_FILE", "CUTOKYO_DATA_DIR", "CUTOKYO_PROXY_ENABLED",
        }
        synthetic = dict.fromkeys(excluded, "synthetic-never-inherit")
        synthetic.update(PATH="/usr/bin:/bin", DISPLAY=":synthetic-display")
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, synthetic, clear=True):
            root = Path(temporary)
            env = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", 12345)
            self.assertFalse(excluded.intersection(env))
            self.assertEqual(env["DISPLAY"], ":synthetic-display")
            self.assertEqual(env["HOME"], str(root / "home"))

    def test_wdio_config_imports_without_runner_but_refuses_unisolated_launch(self):
        env = {key: value for key, value in os.environ.items() if not key.startswith("CUTOKYO_")}
        imported = subprocess.run(
            ["node", "--import", "tsx", "--input-type=module", "--eval", 'import { config } from "./wdio.conf.ts"; console.log(config.capabilities[0]["tauri:options"].application);'],
            cwd=NATIVE.UI, env=env, capture_output=True, text=True)
        self.assertEqual(imported.returncode, 0, imported.stderr)
        self.assertIn("/cutokyo-native-package-runner-required", imported.stdout)
        refused = subprocess.run(
            ["node", "--import", "tsx", "--input-type=module", "--eval", 'import { config } from "./wdio.conf.ts"; config.onPrepare();'],
            cwd=NATIVE.UI, env=env, capture_output=True, text=True)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("missing CUTOKYO_DESKTOP_TEST_ROOT", refused.stderr)

    def test_launch_path_comes_from_debian_bundle_not_debug_decoy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "deb-source"
            (source / "DEBIAN").mkdir(parents=True)
            (source / "DEBIAN/control").write_text(
                "Package: cutokyo-native-regression\nVersion: 0.1.0\nArchitecture: all\n"
                "Maintainer: Synthetic QA <qa@example.invalid>\nDescription: synthetic package test\n")
            (source / "usr/bin").mkdir(parents=True)
            executable = source / "usr/bin/cutokyo-desktop"
            executable.write_text("#!/bin/sh\nprintf 'bundled-only\\n'\n")
            executable.chmod(0o700)
            bundles = root / "target/debug/bundle/deb"
            bundles.mkdir(parents=True)
            (root / "target/debug/cutokyo-desktop").write_text("never launch this build executable")
            package = bundles / "test.deb"
            subprocess.run(["dpkg-deb", "--build", str(source), str(package)], check=True)
            actual_package, application = NATIVE.extract_package(root, dict(os.environ))
            self.assertEqual(actual_package, package)
            self.assertEqual(application, root / "package/usr/bin/cutokyo-desktop")
            self.assertEqual(subprocess.check_output([application], text=True), "bundled-only\n")
            self.assertEqual(NATIVE.sha256(application), NATIVE.sha256(executable))

    def test_missing_bundle_never_falls_back_to_source_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "target/debug").mkdir(parents=True)
            (root / "target/debug/cutokyo-desktop").write_text("source executable")
            with self.assertRaisesRegex(RuntimeError, "Expected one built Debian package"):
                NATIVE.extract_package(root, dict(os.environ))

    def test_owned_app_stops_before_cleanup_even_when_runner_fails(self):
        for exit_code in (0, 7):
            with self.subTest(exit_code=exit_code), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                script = root / "spawn.py"
                script.write_text(
                    "import os, pathlib, subprocess, sys\n"
                    "pathlib.Path('group').write_text(str(os.getpgrp()))\n"
                    "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])\n"
                    f"sys.exit({exit_code})\n")
                if exit_code:
                    with self.assertRaises(subprocess.CalledProcessError):
                        NATIVE.run([sys.executable, str(script)], cwd=root, env=dict(os.environ))
                else:
                    NATIVE.run([sys.executable, str(script)], cwd=root, env=dict(os.environ))
                self.assertFalse(NATIVE.live_group(int((root / "group").read_text())))
                self.assertTrue(root.exists(), "app must stop while its root still exists")

    def test_native_assets_never_replace_production_dist(self):
        ui = NATIVE.UI
        subprocess.run(["pnpm", "build"], cwd=ui, check=True)
        def production_hashes():
            return {str(path.relative_to(ui / "dist")): NATIVE.sha256(path)
                    for path in (ui / "dist").rglob("*") if path.is_file()}
        before = production_hashes()
        self.assertTrue(before)
        with tempfile.TemporaryDirectory(prefix="cutokyo-native-test-wdio-") as temporary:
            root = Path(temporary)
            env = NATIVE.build_environment(root)
            subprocess.run(["pnpm", "build:e2e:tauri:ui"], cwd=ui, env=env, check=True)
            self.assertTrue((root / "assets/index.html").is_file())
            self.assertEqual(before, production_hashes())
            native = "\n".join(path.read_text() for path in (root / "assets").rglob("*.js"))
            self.assertIn("__TAURI__", native)
            invalid = dict(env, CUTOKYO_NATIVE_ASSET_DIR=str(ui / "dist"))
            refused = subprocess.run(["pnpm", "build:e2e:tauri:ui"], cwd=ui, env=invalid, capture_output=True, text=True)
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("ui/dist is production-only", refused.stderr + refused.stdout)
            self.assertEqual(before, production_hashes())
        subprocess.run(["pnpm", "check:production-fixtures"], cwd=ui, check=True)


if __name__ == "__main__":
    unittest.main()

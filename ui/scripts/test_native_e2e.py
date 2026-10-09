"""Non-browser regression tests for package provenance and process isolation."""

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from contextlib import ExitStack
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
                self.assertIn(str(NATIVE.UI), NATIVE.tauri_config(root)["build"]["beforeBuildCommand"])
                runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", port)
                self.assertEqual(runtime["CUTOKYO_NATIVE_PORT"], str(port))
                self.assertEqual(runtime["TAURI_WEBDRIVER_PORT"], str(port))
                self.assertEqual(runtime["CUTOKYO_DESKTOP_TEST_MODE"], "1")
                self.assertEqual(runtime["CUTOKYO_DESKTOP_TEST_ROOT"], str(root))
                for key in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"]:
                    self.assertTrue(Path(runtime[key]).is_relative_to(root))
                    self.assertEqual(Path(runtime[key]).stat().st_mode & 0o777, 0o700)
                self.assertEqual(runtime["PATH"].split(os.pathsep)[0], str(root / "bin"))
                if os.environ.get("DISPLAY"):
                    self.assertEqual(runtime["GDK_BACKEND"], "x11")
                    self.assertEqual(runtime["WEBKIT_DISABLE_DMABUF_RENDERER"], "1")

    def test_x11_requires_a_real_display_and_never_invents_one(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"PATH": "/usr/bin", "GDK_BACKEND": "wayland", "WAYLAND_DISPLAY": "synthetic-wayland"}, clear=True):
            root = Path(temporary)
            runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", 12345)
            self.assertNotIn("DISPLAY", runtime)
            self.assertEqual(runtime["GDK_BACKEND"], "wayland")
            self.assertEqual(runtime["WAYLAND_DISPLAY"], "synthetic-wayland")
            self.assertNotIn("WEBKIT_DISABLE_DMABUF_RENDERER", runtime)

    def test_real_x11_display_is_consistent_for_app_and_terminal_without_host_leaks(self):
        excluded = {
            "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "AWS_SECRET_ACCESS_KEY",
            "CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_CONFIG_DIR",
            "CUTOKYO_CONFIG_FILE", "CUTOKYO_DATA_DIR", "LD_PRELOAD", "NODE_OPTIONS",
            "HTTPS_PROXY", "WAYLAND_SOCKET",
        }
        synthetic = dict.fromkeys(excluded, "synthetic-never-inherit")
        synthetic.update({
            "PATH": "/usr/bin:/bin",
            "DISPLAY": ":synthetic-real-display",
            "WAYLAND_DISPLAY": "synthetic-wayland",
            "GDK_BACKEND": "wayland",
            "HYPRLAND_INSTANCE_SIGNATURE": "synthetic-hyprland",
            "XDG_RUNTIME_DIR": "/host/compositor-runtime",
            "HOME": "/operator/home",
            "XDG_CONFIG_HOME": "/operator/config",
            "XDG_DATA_HOME": "/operator/data",
            "XDG_CACHE_HOME": "/operator/cache",
        })
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, synthetic, clear=True), \
                patch.object(NATIVE.shutil, "which") as resolve_cli:
            root = Path(temporary)
            cli = root / "host-hyprctl"
            cli.write_text("#!/bin/sh\nexit 1\n")
            cli.chmod(0o700)
            resolve_cli.return_value = str(cli)
            runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", 12345)
            self.assertEqual(runtime["DISPLAY"], synthetic["DISPLAY"])
            self.assertEqual(runtime["GDK_BACKEND"], "x11")
            self.assertEqual(runtime["WEBKIT_DISABLE_DMABUF_RENDERER"], "1")
            self.assertNotIn("WAYLAND_DISPLAY", runtime)
            self.assertFalse(excluded.intersection(runtime))
            self.assertEqual(runtime["TERMLAUNCHER"], "kitty")
            self.assertEqual(runtime["HYPRLAND_INSTANCE_SIGNATURE"], synthetic["HYPRLAND_INSTANCE_SIGNATURE"])
            self.assertEqual(runtime["CUTOKYO_NATIVE_COMPOSITOR_RUNTIME"], synthetic["XDG_RUNTIME_DIR"])
            self.assertEqual(runtime["CUTOKYO_NATIVE_COMPOSITOR_CLI"], str(cli.resolve()))
            for key in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"]:
                self.assertTrue(Path(runtime[key]).is_relative_to(root))
                self.assertEqual(Path(runtime[key]).stat().st_mode & 0o777, 0o700)
            self.assertEqual(os.environ["WAYLAND_DISPLAY"], synthetic["WAYLAND_DISPLAY"])
            self.assertEqual(os.environ["XDG_RUNTIME_DIR"], synthetic["XDG_RUNTIME_DIR"])

    def test_compositor_environment_preserves_host_socket_only_for_scoped_helper(self):
        synthetic = {
            "PATH": "/usr/bin:/bin",
            "HYPRLAND_INSTANCE_SIGNATURE": "synthetic-hyprland",
            "XDG_RUNTIME_DIR": "/host/compositor-runtime",
            "CUTOKYO_NATIVE_COMPOSITOR_RUNTIME": "/untrusted-preexisting-override",
            "CUTOKYO_NATIVE_COMPOSITOR_CLI": "/untrusted-preexisting-cli",
            "HOME": "/operator/home",
        }
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, synthetic, clear=True), \
                patch.object(NATIVE.shutil, "which") as resolve_cli:
            root = Path(temporary)
            cli = root / "host-hyprctl"
            cli.write_text("#!/bin/sh\nexit 1\n")
            cli.chmod(0o700)
            alias = root / "host-hyprctl-link"
            alias.symlink_to(cli)
            resolve_cli.return_value = str(alias)
            runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", 12345)
            resolve_cli.assert_called_once_with("hyprctl")
            self.assertEqual(runtime["CUTOKYO_NATIVE_COMPOSITOR_CLI"], str(cli.resolve()))
            self.assertEqual(runtime["HYPRLAND_INSTANCE_SIGNATURE"], "synthetic-hyprland")
            self.assertEqual(runtime["CUTOKYO_NATIVE_COMPOSITOR_RUNTIME"], "/host/compositor-runtime")
            self.assertEqual(runtime["XDG_RUNTIME_DIR"], str(root / "runtime"))
            self.assertEqual(runtime["HOME"], str(root / "home"))
            self.assertEqual(os.environ["XDG_RUNTIME_DIR"], "/host/compositor-runtime")
            for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"]:
                self.assertTrue(Path(runtime[key]).is_relative_to(root))

    def test_missing_compositor_environment_is_not_invented(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"PATH": "/usr/bin"}, clear=True), \
                patch.object(NATIVE.shutil, "which", return_value=None) as resolve_cli:
            root = Path(temporary)
            runtime = NATIVE.runtime_environment(root, root / "evidence", root / "package/usr/bin/cutokyo-desktop", 12345)
            resolve_cli.assert_called_once_with("hyprctl")
            self.assertNotIn("HYPRLAND_INSTANCE_SIGNATURE", runtime)
            self.assertNotIn("CUTOKYO_NATIVE_COMPOSITOR_RUNTIME", runtime)
            self.assertNotIn("CUTOKYO_NATIVE_COMPOSITOR_CLI", runtime)
            self.assertEqual(runtime["XDG_RUNTIME_DIR"], str(root / "runtime"))

    def test_trusted_keyboard_ownership_and_refusal_regressions(self):
        subprocess.run(["node", "--import", "tsx", "--test", "tests/tauri/trusted-keyboard.test.ts"],
                       cwd=NATIVE.UI, check=True)

    def test_compositor_resizes_only_the_extracted_app_pid_and_address(self):
        with patch.dict(os.environ, {"HYPRLAND_INSTANCE_SIGNATURE": "synthetic"}):
            clients = [
                {"pid": 9999, "address": "0xabc1", "floating": False},
                {"pid": 1234, "address": "0xabc2", "floating": False},
            ]
            commands = []
            def record(command, **_kwargs):
                commands.append(command)
                return subprocess.CompletedProcess(command, 0)
            with patch.object(NATIVE, "isolated_app_pid", return_value=1234), \
                    patch.object(NATIVE.subprocess, "check_output", return_value=json.dumps(clients)), \
                    patch.object(NATIVE.subprocess, "run", side_effect=record):
                state = {}
                NATIVE.size_isolated_window(Path("/isolated/package/usr/bin/cutokyo-desktop"), threading.Event(), state)
            self.assertEqual(state, {"pid": 1234, "window": "0xabc2"})
            self.assertEqual(commands, [
                ["hyprctl", "dispatch", "togglefloating", "address:0xabc2"],
                ["hyprctl", "dispatch", "resizewindowpixel", "exact 1280 800,address:0xabc2"],
            ])
            self.assertNotIn("0xabc1", repr(commands))

    def test_only_the_exact_package_executable_can_be_selected(self):
        self.assertIsNotNone(NATIVE.isolated_app_pid(Path(os.readlink(f"/proc/{os.getpid()}/exe"))))
        self.assertIsNone(NATIVE.isolated_app_pid(Path("/nonexistent/cutokyo-desktop")))

    def test_unmatched_or_ambiguous_compositor_windows_are_never_resized(self):
        self.assertIsNone(NATIVE.isolated_window([{"pid": 9999, "address": "0x1"}], 1234))
        with self.assertRaisesRegex(RuntimeError, "multiple compositor windows"):
            NATIVE.isolated_window([{"pid": 1234}, {"pid": 1234}], 1234)
        with patch.dict(os.environ, {"HYPRLAND_INSTANCE_SIGNATURE": "synthetic"}), \
                patch.object(NATIVE, "isolated_app_pid", return_value=1234), \
                patch.object(NATIVE.subprocess, "check_output", return_value='[{"pid":1234,"address":"malformed","floating":false}]'), \
                patch.object(NATIVE.subprocess, "run") as dispatch:
            state = {}
            NATIVE.size_isolated_window(Path("/isolated/app"), threading.Event(), state)
            self.assertIsInstance(state.get("error"), RuntimeError)
            dispatch.assert_not_called()

    def test_runner_refuses_an_app_that_survives_the_phase(self):
        def observed(_application, _done, state):
            del _application, _done
            state["pid"] = 1234
        with patch.object(NATIVE, "size_isolated_window", side_effect=observed), \
                patch.object(NATIVE, "run"), \
                patch.object(NATIVE, "isolated_app_pid", return_value=1234):
            with self.assertRaisesRegex(RuntimeError, "survived"):
                NATIVE.run_native_phase(Path("/isolated/app"), {}, "save")

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

    def test_completed_capture_management_arranges_ui_save_before_missing_config_snapshot(self):
        # Execute only the maintained case's arrangement against stubbed WDIO
        # boundaries. No app config is fabricated: stop at save acknowledgement.
        script = '''
            import assert from "node:assert/strict";
            import { _setGlobal } from "@wdio/globals";
            let journey;
            let checked = true;
            const actions = [];
            const stop = new Error("arrangement reached acknowledged Settings save");
            globalThis.describe = (_title, register) => register();
            globalThis.it = (title, body) => {
                if (title.startsWith("manages completed-user capture through Settings")) journey = body;
            };
            _setGlobal("browser", {
                waitUntil: async (condition) => assert.equal(await condition(), true),
            }, false);
            _setGlobal("$", (selector) => {
                if (selector === "a=Settings") return {
                    click: async () => actions.push("open Settings"),
                };
                if (selector === "h1=Settings") return {
                    waitForDisplayed: async () => actions.push("Settings ready"),
                };
                if (selector === 'input[aria-label="Agent search"]') return {
                    waitForDisplayed: async () => actions.push("Search MCP ready"),
                    isSelected: async () => checked,
                    click: async () => { checked = false; actions.push("save nondefault Search MCP"); },
                };
                if (selector === 'main .success-message[role="status"]') return {
                    waitForDisplayed: async () => {
                        assert.equal(checked, false);
                        assert.deepEqual(actions, ["open Settings", "Settings ready", "Search MCP ready", "save nondefault Search MCP"]);
                        throw stop;
                    },
                };
                throw new Error("Unexpected pre-snapshot UI operation: " + selector);
            }, false);
            await import("./tests/tauri/native-desktop.spec.ts");
            assert.equal(typeof journey, "function");
            await assert.rejects(journey(), (error) => error === stop);
            console.log(stop.message);
        '''
        with tempfile.TemporaryDirectory(prefix="cutokyo-native-test-wdio-") as temporary:
            root = Path(temporary)
            (root / "data").mkdir()
            (root / "data/desktop-state.json").write_text(json.dumps({
                "onboarding_complete": True,
                "selected_harnesses": ["claude_code", "codex", "opencode"],
            }))
            config = root / "config/config.toml"
            self.assertFalse(config.exists())
            env = {key: value for key, value in os.environ.items() if not key.startswith("CUTOKYO_")}
            env.update(CUTOKYO_NATIVE_PHASE="capture-setup", CUTOKYO_DESKTOP_TEST_ROOT=str(root),
                       CUTOKYO_NATIVE_EVIDENCE=str(root / "evidence"))
            result = subprocess.run(
                ["node", "--import", "tsx", "--input-type=module", "--eval", script],
                cwd=NATIVE.UI, env=env, capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("arrangement reached acknowledged Settings save", result.stdout)
            self.assertFalse(config.exists(), "Boundary regression must not fabricate app config")

    def test_phase_selection_registers_every_applicable_journey_without_running_it(self):
        backup = "./tests/tauri/backup-restore.spec.ts"
        inventory = "./tests/tauri/inventory-management.spec.ts"
        desktop = "./tests/tauri/native-desktop.spec.ts"
        health = "./tests/tauri/zz-health-capabilities.spec.ts"
        expected = {
            "save": {
                backup: [
                    "creates, cancels safely, restores actual history, and retains verified prior history",
                    "reports tampering inside the restore modal without replacing native history",
                ],
                inventory: [
                    "edits, copies all skill assets, then removes only the selected native bundle",
                    "rejects stale and malformed MCP edits, converts real config credentials, and protects capture hooks",
                ],
                desktop: [
                    "searches, opens, exactly resumes, and deletes an isolated native fixture",
                    "saves appearance to native desktop state",
                ],
                health: [
                    "keeps trusted keyboard skip navigation on the current native route",
                    "discloses unavailable signed updates before dead-end actions",
                    "cancels safely then exports exactly the previewed private diagnostic report",
                ],
            },
            "reopen": {
                backup: ["rediscovers the private backup after a real application restart"],
                inventory: ["rediscovers persisted native edits/copies without resurrecting deleted bundles"],
                desktop: ["restores saved appearance after a native application restart"],
            },
            "browse": {
                desktop: ["browses a real packaged application without installing native capture"],
            },
            "state-recovery": {
                desktop: ["preserves rejected desktop preferences and clears their single warning after explicit repair"],
            },
            "capture-setup": {
                desktop: [
                    "previews, cancels, installs, verifies, recovers and removes each real isolated native integration",
                    "manages completed-user capture through Settings without resetting saved intent or unmanaged native bytes",
                ],
            },
            "capture-history": {
                desktop: ["opens real native-hook and transcript evidence and resumes its exact private native context"],
            },
        }
        script = r'''
            import { globSync } from "node:fs";
            import { pathToFileURL } from "node:url";
            import { resolve } from "node:path";
            import { config } from "./wdio.conf.ts";
            const registrations = {};
            let current;
            globalThis.describe = (_title, register) => register();
            // Register actual maintained suites, but never execute any journey body.
            globalThis.it = (title, body) => {
                if (typeof body !== "function") throw new Error("Missing journey body");
                registrations[current].push(title);
            };
            for (const spec of config.specs) {
                const files = [...globSync(spec)];
                if (!files.length) throw new Error("No existing spec: " + spec);
                for (const file of files) {
                    current = file.startsWith("./") ? file : "./" + file;
                    registrations[current] = [];
                    await import(pathToFileURL(resolve(file)).href);
                    if (!registrations[current].length) throw new Error("Zero-case spec: " + file);
                }
            }
            if (!Object.keys(registrations).length) throw new Error("Zero selected specs");
            console.log(JSON.stringify({ selection: config.specs, registrations }));
        '''
        env = {key: value for key, value in os.environ.items() if not key.startswith("CUTOKYO_")}
        env.update(CUTOKYO_DESKTOP_TEST_ROOT="/registration-only/no-application",
                   CUTOKYO_NATIVE_EVIDENCE="/registration-only/no-evidence")
        registered_cases = []
        for phase, suites in expected.items():
            with self.subTest(phase=phase):
                result = subprocess.run(
                    ["node", "--import", "tsx", "--input-type=module", "--eval", script],
                    cwd=NATIVE.UI, env={**env, "CUTOKYO_NATIVE_PHASE": phase},
                    capture_output=True, text=True, timeout=15)
                self.assertEqual(result.returncode, 0, result.stderr)
                registered = json.loads(result.stdout)
                self.assertEqual(registered["selection"], list(suites), "Explicit spec selection changed")
                actual = registered["registrations"]
                self.assertEqual(list(actual), list(suites), "Spec order/coverage changed")
                self.assertEqual(actual, suites)
                registered_cases.extend(title for titles in actual.values() for title in titles)
        self.assertEqual(len(registered_cases), 17)
        self.assertEqual(len(set(registered_cases)), 17, "A journey was duplicated across phases")
        self.assertEqual({str(path.relative_to(NATIVE.UI)) for path in (NATIVE.UI / "tests/tauri").glob("*.spec.ts")},
                         {spec.removeprefix("./") for suites in expected.values() for spec in suites},
                         "A maintained spec is missing from phase coverage")

    def test_phase_selection_rejects_missing_unknown_and_empty_before_launch(self):
        env = {key: value for key, value in os.environ.items() if not key.startswith("CUTOKYO_")}
        script = 'import { config } from "./wdio.conf.ts"; console.log(JSON.stringify(config.specs));'
        for phase in (None, "", "unknown", "SAVE", "save ", "__proto__", "constructor", "toString"):
            with self.subTest(phase=phase):
                selected_env = dict(env)
                if phase is not None:
                    selected_env["CUTOKYO_NATIVE_PHASE"] = phase
                result = subprocess.run(
                    ["node", "--import", "tsx", "--input-type=module", "--eval", script],
                    cwd=NATIVE.UI, env=selected_env, capture_output=True, text=True, timeout=15)
                self.assertNotEqual(result.returncode, 0, "Invalid phase selected specs without refusal")
                self.assertIn("CUTOKYO_NATIVE_PHASE", result.stderr)
                self.assertEqual(result.stdout, "")

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


class NativeResumeTerminalTeardown(unittest.TestCase):
    """Mock only OS/compositor boundaries; never launch or signal a real terminal."""

    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="cutokyo-native-test-wdio-")))
        host = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        for name in ("home", "runtime", "data"):
            (self.root / name).mkdir(mode=0o700)
        self.project = self.root / "data/project with spaces ' and $literal"
        self.project.mkdir(mode=0o700)
        application = self.root / "package/usr/bin/cutokyo-desktop"
        receiver = self.root / "bin/cutokyo"
        self.kitty = host / "kitty"
        for path in (application, receiver, self.kitty):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic executable, never launched")
            path.chmod(0o700)
        self.native_id = "claude-native-73A9"
        audit = self.root / "resume-audit.txt"
        audit.write_text(f"--resume\n{self.native_id}\n")
        audit.with_suffix(".context.json").write_text(json.dumps({
            "cwd": str(self.project), "stdinTty": True, "stdoutTty": True}))
        self.env = {"HOME": str(self.root / "home"), "XDG_RUNTIME_DIR": str(self.root / "runtime"),
                    "CUTOKYO_DESKTOP_TEST_ROOT": str(self.root), "CUTOKYO_DESKTOP_TEST_MODE": "1",
                    "CUTOKYO_NATIVE_APPLICATION": str(application), "CUTOKYO_NATIVE_RESUME_AUDIT": str(audit)}
        self.processes = [
            {"pid": 10, "parent": 1, "group": 777, "start": 1, "exe": "/usr/bin/node", "argv": ["node", "wdio"]},
            {"pid": 20, "parent": 10, "group": 777, "start": 2, "exe": str(application), "argv": [str(application)]},
            {"pid": 30, "parent": 20, "group": 777, "start": 3, "exe": str(self.kitty), "argv": [
                str(self.kitty), "--title=Cutokyo native session resume", "--working-directory", str(self.project),
                "--hold", "--", str(receiver), "--data-dir", str(self.root / "data"), "resume-terminal", "--request",
                str(self.root / "data/resume-launch-unique/request.json")]},
            {"pid": 31, "parent": 30, "group": 777, "start": 4, "exe": str(host / "kitten"), "argv": [str(host / "kitten"), "__atexit__"]},
            {"pid": 32, "parent": 30, "group": 32, "start": 4, "exe": str(Path("/bin/bash").resolve()), "argv": ["/bin/bash", "--posix"]},
        ]
        self.clients = [{"pid": 20, "address": "0xabc1", "mapped": True, "xwayland": True},
                        {"pid": 30, "address": "0xabc2", "mapped": True, "xwayland": True}]
        self.active = copy.deepcopy(self.clients[1])
        self.environments = {process["pid"]: self.env.copy() for process in self.processes}
        self.stopped = False
        self.stack.enter_context(patch.object(NATIVE.shutil, "which", return_value=str(self.kitty)))
        self.stack.enter_context(patch.object(NATIVE.os, "getpgrp", return_value=777))
        self.stack.enter_context(patch.object(NATIVE, "resume_processes", side_effect=lambda:
            copy.deepcopy(self.processes[:2] if self.stopped else self.processes)))
        self.stack.enter_context(patch.object(NATIVE, "resume_compositor", side_effect=lambda _env, query:
            copy.deepcopy(self.clients[:1] if self.stopped else self.clients) if query == "clients" else copy.deepcopy(self.active)))
        original_read = Path.read_bytes
        def read_bytes(path):
            if path.name == "environ" and path.parent.parent == Path("/proc"):
                return b"\0".join(f"{key}={value}".encode() for key, value in self.environments[int(path.parent.name)].items())
            return original_read(path)
        self.stack.enter_context(patch.object(Path, "read_bytes", read_bytes))
        self.open_pidfd = self.stack.enter_context(patch.object(NATIVE.os, "pidfd_open", return_value=99))
        self.close_pidfd = self.stack.enter_context(patch.object(NATIVE.os, "close"))
        def stop(_descriptor, _signal):
            self.stopped = True
        self.signal = self.stack.enter_context(patch.object(NATIVE.signal, "pidfd_send_signal", side_effect=stop))
        self.select = self.stack.enter_context(patch("select.select", side_effect=lambda *_args:
            ([99], [], []) if self.stopped else ([], [], [])))

    def proof(self):
        return NATIVE.resume_terminal_proof(self.project, self.native_id, self.env)

    def test_closes_only_completed_owned_terminal_via_pidfd_and_preserves_application(self):
        proof = self.proof()
        self.assertTrue(proof["ready"])
        receipt = NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.open_pidfd.assert_called_once_with(30)
        self.signal.assert_called_once_with(99, NATIVE.signal.SIGTERM)
        self.close_pidfd.assert_called_once_with(99)
        self.assertEqual(receipt["before"]["terminal"]["start"], 3)
        self.assertEqual(receipt["before"]["window"]["address"], "0xabc2")
        for key in ("terminalExited", "descendantsExited", "windowRemoved", "applicationPreserved"):
            self.assertIs(receipt[key], True)
        self.assertEqual(receipt["keyboardDispatches"], [])
        self.assertFalse(receipt["focusChangedByHelper"])

    def test_foreign_ambiguous_stale_and_unfinished_actors_refuse_before_signal(self):
        original = copy.deepcopy((self.processes, self.clients, self.active, self.environments, self.env))
        cases = {
            "duplicate application": lambda: self.processes.append({**self.processes[1], "pid": 21}),
            "duplicate terminal": lambda: self.processes.append({**self.processes[2], "pid": 33}),
            "foreign parent": lambda: self.processes[2].update(parent=999),
            "foreign group": lambda: self.processes[2].update(group=888),
            "foreign app group": lambda: self.processes[1].update(group=888),
            "app owner absent": lambda: self.processes.pop(0),
            "unexpected app argv": lambda: self.processes[1]["argv"].append("--another-mode"),
            "stale start identity": lambda: self.processes[2].update(start=5),
            "foreign working directory": lambda: self.processes[2]["argv"].__setitem__(3, "/operator/project"),
            "foreign request": lambda: self.processes[2]["argv"].__setitem__(-1, "/operator/request.json"),
            "foreign process HOME": lambda: self.environments[30].update(HOME="/operator/home"),
            "foreign runtime": lambda: self.environments[20].update(XDG_RUNTIME_DIR="/host/runtime"),
            "foreign application": lambda: self.env.update(CUTOKYO_NATIVE_APPLICATION="/operator/application"),
            "wrong mode": lambda: self.env.update(CUTOKYO_DESKTOP_TEST_MODE="0"),
            "foreign active window": lambda: self.active.update(pid=999, address="0x999"),
            "ambiguous window": lambda: self.clients.append({**self.clients[1], "pid": 999}),
            "multiple terminal windows": lambda: self.clients.append({**self.clients[1], "address": "0xabc3"}),
            "unmapped terminal": lambda: self.clients[1].update(mapped=False),
            "invalid address": lambda: self.clients[1].update(address="0xabc2,all"),
            "unfinished helper": lambda: self.processes[4].update(exe=str(self.root / "bin/cutokyo")),
            "unfinished harness": lambda: self.processes[4].update(exe=str(self.root / "bin/claude")),
            "unexpected descendant": lambda: self.processes[4].update(argv=["bash", "-c", "still working"]),
        }
        for name, mutate in cases.items():
            with self.subTest(name=name):
                self.processes, self.clients, self.active, self.environments, self.env = copy.deepcopy(original)
                proof = self.proof()
                mutate()
                with self.assertRaises(RuntimeError):
                    NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
                self.open_pidfd.assert_not_called()
                self.signal.assert_not_called()

    def test_live_native_child_is_pending_not_cleanup_success(self):
        self.processes[4]["exe"] = str(self.root / "bin/claude")
        proof = self.proof()
        self.assertFalse(proof["ready"])
        with self.assertRaisesRegex(RuntimeError, "unfinished"):
            NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.signal.assert_not_called()

    def test_pidfd_acquisition_race_or_exited_handle_never_signals(self):
        proof = self.proof()
        def changed(_pid):
            self.processes[2]["start"] = 9
            return 99
        self.open_pidfd.side_effect = changed
        with self.assertRaisesRegex(RuntimeError, "identity changed"):
            NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.signal.assert_not_called()
        self.close_pidfd.assert_called_once_with(99)
        self.processes[2]["start"] = 3
        self.open_pidfd.side_effect = None
        self.select.side_effect = None
        self.select.return_value = ([99], [], [])
        with self.assertRaisesRegex(RuntimeError, "identity changed"):
            NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.signal.assert_not_called()

    def test_unavailable_pidfd_and_exit_timeout_have_no_broader_fallback(self):
        proof = self.proof()
        self.open_pidfd.side_effect = OSError("pidfd unavailable")
        with self.assertRaises(OSError):
            NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.signal.assert_not_called()
        self.open_pidfd.side_effect = None
        self.select.side_effect = None
        self.select.return_value = ([], [], [])
        with self.assertRaisesRegex(RuntimeError, "no broader signal"):
            NATIVE.cleanup_resume_terminal(self.project, self.native_id, proof, self.env)
        self.signal.assert_called_once_with(99, NATIVE.signal.SIGTERM)

    def test_symlink_audit_or_nonprivate_directory_refuses_without_signal(self):
        audit = self.root / "resume-audit.txt"
        saved = self.root / "saved-audit"
        audit.rename(saved)
        audit.symlink_to(saved)
        with self.assertRaisesRegex(RuntimeError, "non-symlink audits"):
            self.proof()
        audit.unlink()
        saved.rename(audit)
        (self.root / "runtime").chmod(0o755)
        with self.assertRaisesRegex(RuntimeError, "owner-only"):
            self.proof()
        self.signal.assert_not_called()


if __name__ == "__main__":
    unittest.main()

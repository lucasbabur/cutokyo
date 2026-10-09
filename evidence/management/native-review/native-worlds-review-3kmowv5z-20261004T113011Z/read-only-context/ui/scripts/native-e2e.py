#!/usr/bin/env python3
"""Build a test-only Debian bundle, then drive its extracted executable with WDIO.

No source/debug executable is accepted. Each invocation owns its build, assets,
ports, HOME and data. WDIO and all its children stop before those roots disappear.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from typing import TypedDict


class ResumeProcess(TypedDict):
    pid: int
    parent: int
    group: int
    start: int
    exe: str
    argv: list[str]


class ResumeWindow(TypedDict):
    pid: int
    address: str
    mapped: bool
    xwayland: bool


class ResumeTerminalProof(TypedDict):
    ready: bool
    app: ResumeProcess
    terminal: ResumeProcess
    window: ResumeWindow
    descendants: list[ResumeProcess]

UI = Path(__file__).resolve().parents[1]
REPOSITORY = UI.parent


def live_group(group: int) -> bool:
    """Ignore already-exited zombies; never select processes by name or port."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
            if int(fields[2]) == group and fields[0] != "Z":
                return True
        except (FileNotFoundError, ProcessLookupError):
            continue
    return False


def stop_group(group: int) -> None:
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(group, sig)
        except ProcessLookupError:
            return
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if not live_group(group):
                return
            time.sleep(0.05)
    raise RuntimeError("Owned native process group did not stop; retaining test roots")


def run(command: list[str], *, cwd: Path, env: dict[str, str]) -> None:
    print(json.dumps({"command": command, "cwd": str(cwd)}), flush=True)
    process = subprocess.Popen(command, cwd=cwd, env=env, start_new_session=True)
    try:
        code = process.wait()
    finally:
        # WDIO hooks may run before service teardown, or startup may throw. The
        # outer owner is the only place allowed to remove app data/package files.
        stop_group(process.pid)
        process.wait()
    if code:
        raise subprocess.CalledProcessError(code, command)


def reserve_port() -> socket.socket:
    reservation = socket.socket()
    reservation.bind(("127.0.0.1", 0))
    reservation.listen(1)
    return reservation


def build_environment(root: Path) -> dict[str, str]:
    return {
        **os.environ,
        "CARGO_TARGET_DIR": str(root / "target"),
        "CUTOKYO_NATIVE_ASSET_DIR": str(root / "assets"),
    }


def tauri_config(root: Path) -> dict[str, object]:
    return {
        "build": {
            "beforeBuildCommand": f"pnpm --dir {shlex.quote(str(UI))} build:e2e:tauri:ui",
            "frontendDist": str(root / "assets"),
        },
    }


def extract_package(root: Path, env: dict[str, str]) -> tuple[Path, Path]:
    packages = list((root / "target/debug/bundle/deb").glob("*.deb"))
    if len(packages) != 1:
        raise RuntimeError(f"Expected one built Debian package, found {len(packages)}")
    package = packages[0]
    destination = root / "package"
    run(["dpkg-deb", "--extract", str(package), str(destination)], cwd=root, env=env)
    application = destination / "usr/bin/cutokyo-desktop"
    if application.is_symlink() or not application.is_file() or not os.access(application, os.X_OK):
        raise RuntimeError("The Debian bundle has no regular executable desktop application")
    return package, application


def runtime_environment(root: Path, evidence: Path, application: Path, port: int) -> dict[str, str]:
    # Keep only process/display plumbing, not provider credentials, operator
    # harness locations, fixture selectors, proxy settings or loader hooks.
    allowed = {
        "PATH", "DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS",
        "LANG", "LC_ALL", "TZ", "GDK_BACKEND", "LIBGL_ALWAYS_SOFTWARE",
        "WEBKIT_DISABLE_COMPOSITING_MODE", "WEBKIT_DISABLE_DMABUF_RENDERER",
        "HYPRLAND_INSTANCE_SIGNATURE",
    }
    env = {key: value for key, value in os.environ.items() if key in allowed}
    if host_runtime := os.environ.get("XDG_RUNTIME_DIR"):
        env["CUTOKYO_NATIVE_COMPOSITOR_RUNTIME"] = host_runtime
    if compositor_cli := shutil.which("hyprctl"):
        env["CUTOKYO_NATIVE_COMPOSITOR_CLI"] = str(Path(compositor_cli).resolve(strict=True))
    for name in ("home", "config", "data", "cache", "runtime"):
        (root / name).mkdir(mode=0o700)
    # WebKitGTK's embedded driver can disappear during a Wayland navigation on
    # this compositor. XWayland keeps the real packaged app and real WebDriver,
    # while avoiding that renderer path; never invent a DISPLAY on pure Wayland.
    if env.get("DISPLAY"):
        env["GDK_BACKEND"] = "x11"
        # Kitty selects Wayland from WAYLAND_DISPLAY independently of GTK. The
        # app's private runtime has no host Wayland socket; use the real X display
        # consistently for the app and its visible native-resume terminal.
        env.pop("WAYLAND_DISPLAY", None)
        env["WEBKIT_DISABLE_DMABUF_RENDERER"] = "1"
    env.update({
        "HOME": str(root / "home"),
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_DATA_HOME": str(root / "data"),
        "XDG_CACHE_HOME": str(root / "cache"),
        "XDG_RUNTIME_DIR": str(root / "runtime"),
        "CUTOKYO_DESKTOP_TEST_MODE": "1",
        "CUTOKYO_DESKTOP_TEST_ROOT": str(root),
        "CUTOKYO_DESKTOP_TEST_FIXTURE": "search-resume",
        "CUTOKYO_NATIVE_RESUME_AUDIT": str(root / "resume-audit.txt"),
        "TERMLAUNCHER": "kitty",
        "CUTOKYO_NATIVE_APPLICATION": str(application),
        "CUTOKYO_NATIVE_EVIDENCE": str(evidence),
        "CUTOKYO_NATIVE_PORT": str(port),
        "TAURI_WEBDRIVER_PORT": str(port),
        "PATH": str(root / "bin") + os.pathsep + env.get("PATH", ""),
    })
    return env


def resume_processes() -> list[ResumeProcess]:
    """Read identities, never select a process from its name or window title."""
    processes: list[ResumeProcess] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
            if fields[0] == "Z":
                continue
            processes.append({
                "pid": int(entry.name), "parent": int(fields[1]),
                "group": int(fields[2]), "start": int(fields[19]),
                "exe": os.readlink(entry / "exe"),
                "argv": [arg.decode() for arg in (entry / "cmdline").read_bytes().split(b"\0") if arg],
            })
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return processes


def private_resume_process(process: ResumeProcess, root: Path) -> None:
    values = dict(entry.decode().split("=", 1) for entry in
                  (Path("/proc") / str(process["pid"]) / "environ").read_bytes().split(b"\0") if entry)
    required = {"HOME": str(root / "home"), "XDG_RUNTIME_DIR": str(root / "runtime"),
                "CUTOKYO_DESKTOP_TEST_ROOT": str(root), "CUTOKYO_DESKTOP_TEST_MODE": "1"}
    if any(values.get(key) != value for key, value in required.items()):
        raise RuntimeError("Held terminal cleanup refused a non-private process environment")


def resume_compositor(env: dict[str, str], query: str) -> object:
    cli = Path(env["CUTOKYO_NATIVE_COMPOSITOR_CLI"])
    runtime = Path(env["CUTOKYO_NATIVE_COMPOSITOR_RUNTIME"])
    if not cli.is_absolute() or cli.resolve(strict=True) != cli or not cli.is_file() or not os.access(cli, os.X_OK):
        raise RuntimeError("Held terminal cleanup requires a canonical executable compositor CLI")
    if not runtime.is_absolute() or str(runtime) == env["XDG_RUNTIME_DIR"] or not env.get("HYPRLAND_INSTANCE_SIGNATURE"):
        raise RuntimeError("Held terminal cleanup requires separate host compositor plumbing")
    return json.loads(subprocess.check_output([str(cli), query, "-j"],
        env={**env, "XDG_RUNTIME_DIR": str(runtime)}, text=True, timeout=5))


def resume_window(value: object) -> ResumeWindow:
    if not isinstance(value, dict):
        raise RuntimeError("Held terminal cleanup refused invalid compositor window")
    pid, address = value.get("pid"), value.get("address")
    mapped, xwayland = value.get("mapped"), value.get("xwayland")
    if (not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0
            or not isinstance(address, str) or not address.startswith("0x") or len(address) < 3
            or any(char not in "0123456789abcdefABCDEF" for char in address[2:])
            or not isinstance(mapped, bool) or not isinstance(xwayland, bool)):
        raise RuntimeError("Held terminal cleanup refused invalid compositor window identity")
    return {"pid": pid, "address": address, "mapped": mapped, "xwayland": xwayland}


def resume_terminal_proof(project: Path, native_id: str, env: dict[str, str]) -> ResumeTerminalProof:
    root = Path(env["CUTOKYO_DESKTOP_TEST_ROOT"])
    if (env.get("CUTOKYO_DESKTOP_TEST_MODE") != "1" or not root.is_absolute()
            or root.resolve(strict=True) != root or root.parent != Path(tempfile.gettempdir()).resolve()
            or not root.name.startswith("cutokyo-native-test-wdio-")):
        raise RuntimeError("Held terminal cleanup requires the exact private temporary test root")
    application = root / "package/usr/bin/cutokyo-desktop"
    receiver = root / "bin/cutokyo"
    kitty = Path(shutil.which("kitty") or "")
    for executable in (application, receiver, kitty):
        if not executable.is_absolute() or executable.resolve(strict=True) != executable or not executable.is_file() or not os.access(executable, os.X_OK):
            raise RuntimeError("Held terminal cleanup requires canonical matching executables")
    if env.get("CUTOKYO_NATIVE_APPLICATION") != str(application) or env.get("HOME") != str(root / "home") or env.get("XDG_RUNTIME_DIR") != str(root / "runtime"):
        raise RuntimeError("Held terminal cleanup refused foreign application or runtime paths")
    if not project.is_absolute() or project.resolve(strict=True) != project or not project.is_relative_to(root):
        raise RuntimeError("Held terminal cleanup requires the audited private project")
    for directory in (root, root / "home", root / "runtime", root / "data"):
        if directory.resolve(strict=True) != directory or not directory.is_dir() or directory.stat().st_mode & 0o777 != 0o700:
            raise RuntimeError("Held terminal cleanup requires canonical owner-only private directories")
    audit = root / "resume-audit.txt"
    if any(path.is_symlink() or not path.is_file() for path in (audit, audit.with_suffix(".context.json"))):
        raise RuntimeError("Held terminal cleanup requires regular non-symlink audits")
    if env.get("CUTOKYO_NATIVE_RESUME_AUDIT") != str(audit) or audit.read_text() != f"--resume\n{native_id}\n":
        raise RuntimeError("Held terminal cleanup requires the completed exact native argv audit")
    if json.loads(audit.with_suffix(".context.json").read_text()) != {"cwd": str(project), "stdinTty": True, "stdoutTty": True}:
        raise RuntimeError("Held terminal cleanup requires the completed exact project and TTY audit")
    processes = resume_processes()
    apps = [process for process in processes if process["exe"] == str(application)]
    if len(apps) != 1 or apps[0]["group"] != os.getpgrp():
        raise RuntimeError("Held terminal cleanup requires one packaged app in the owned runner group")
    app = apps[0]
    owners = [process for process in processes if process["pid"] == app["parent"] and process["group"] == app["group"]]
    if len(owners) != 1 or app["argv"] != [str(application)]:
        raise RuntimeError("Held terminal cleanup refused foreign packaged app parent or argv")
    private_resume_process(owners[0], root)
    private_resume_process(app, root)
    candidates = [process for process in processes if process["exe"] == str(kitty)
                  and process["parent"] == app["pid"]]
    if len(candidates) != 1:
        raise RuntimeError("Held terminal cleanup requires one direct packaged-app terminal child")
    terminal = candidates[0]
    private_resume_process(terminal, root)
    if not 0 < app["start"] <= terminal["start"]:
        raise RuntimeError("Held terminal cleanup refused invalid process start identities")
    argv = terminal["argv"]
    prefix = [str(kitty), "--title=Cutokyo native session resume", "--working-directory", str(project),
              "--hold", "--", str(receiver), "--data-dir", str(root / "data"), "resume-terminal", "--request"]
    if terminal["group"] != app["group"] or len(argv) != len(prefix) + 1 or argv[:-1] != prefix:
        raise RuntimeError("Held terminal cleanup refused foreign group or launch argv")
    request = Path(argv[-1])
    if (request.name != "request.json" or request.parent.parent != root / "data"
            or not request.parent.name.startswith("resume-launch-") or request.resolve() != request):
        raise RuntimeError("Held terminal cleanup refused a foreign request path")
    descendants: list[ResumeProcess] = []
    parents = {terminal["pid"]}
    while children := [process for process in processes if process["parent"] in parents and process not in descendants]:
        descendants.extend(children)
        parents.update(process["pid"] for process in children)
    ready = True
    for child in descendants:
        private_resume_process(child, root)
        if child["exe"] in {str(receiver), str(root / "bin/claude")} or any(str(receiver) == arg for arg in child["argv"]):
            ready = False
        elif (child["exe"], child["argv"]) not in [
                (str(kitty.parent / "kitten"), [str(kitty.parent / "kitten"), "__atexit__"]),
                (str(Path("/bin/bash").resolve()), ["/bin/bash", "--posix"])]:
            raise RuntimeError("Held terminal cleanup refused an unexpected or unfinished descendant")
    raw_clients = resume_compositor(env, "clients")
    if not isinstance(raw_clients, list):
        raise RuntimeError("Held terminal cleanup refused invalid compositor state")
    clients = [resume_window(value) for value in raw_clients]
    active = resume_window(resume_compositor(env, "activewindow"))
    windows = [window for window in clients if window["pid"] == terminal["pid"]]
    app_windows = [window for window in clients if window["pid"] == app["pid"]]
    if len(windows) != 1 or len(app_windows) != 1 or not app_windows[0]["mapped"]:
        raise RuntimeError("Held terminal cleanup requires unique owned app and terminal windows")
    window = windows[0]
    if (sum(candidate["address"] == window["address"] for candidate in clients) != 1
            or not window["mapped"] or not window["xwayland"]
            or active != window):
        raise RuntimeError("Held terminal cleanup refused foreign, ambiguous or stale active window")
    return {"ready": ready, "app": app, "terminal": terminal,
            "window": window, "descendants": descendants}


def cleanup_resume_terminal(project: Path, native_id: str, expected: ResumeTerminalProof, env: dict[str, str]) -> dict[str, object]:
    """Signal only the proved actor via pidfd; never focus, dispatch keys or kill a group."""
    import select
    proof = resume_terminal_proof(project, native_id, env)
    if proof != expected or proof["ready"] is not True:
        raise RuntimeError("Held terminal cleanup refused stale identity or unfinished native execution")
    descriptor = os.pidfd_open(proof["terminal"]["pid"])
    try:
        if resume_terminal_proof(project, native_id, env) != proof or select.select([descriptor], [], [], 0)[0]:
            raise RuntimeError("Held terminal cleanup identity changed after pidfd acquisition")
        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        if not select.select([descriptor], [], [], 5)[0]:
            raise RuntimeError("Owned held terminal did not exit; no broader signal is permitted")
    finally:
        os.close(descriptor)
    deadline = time.monotonic() + 5
    while True:
        remaining = resume_processes()
        if proof["app"] not in remaining:
            raise RuntimeError("Packaged application identity changed during held terminal cleanup")
        raw_clients = resume_compositor(env, "clients")
        if not isinstance(raw_clients, list):
            raise RuntimeError("Held terminal cleanup refused invalid final compositor state")
        clients = [resume_window(value) for value in raw_clients]
        terminal_gone = not any(process["pid"] == proof["terminal"]["pid"] for process in remaining)
        descendants_gone = not any(process["pid"] in {child["pid"] for child in proof["descendants"]} for process in remaining)
        window_gone = not any(window.get("pid") == proof["terminal"]["pid"] or window.get("address") == proof["window"]["address"] for window in clients)
        if terminal_gone and descendants_gone and window_gone:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("Owned held terminal descendants/window survived cleanup")
        threading.Event().wait(0.02)
    return {"verdict": "PASS", "before": proof, "signal": "SIGTERM via pidfd",
            "terminalExited": True, "descendantsExited": True, "windowRemoved": True,
            "applicationPreserved": True, "keyboardDispatches": [], "focusChangedByHelper": False}


def resume_cleanup_command(arguments: list[str]) -> None:
    operation, project, native_id, *encoded = arguments
    receipt: object
    if operation == "--inspect-resume-terminal" and not encoded:
        receipt = resume_terminal_proof(Path(project), native_id, dict(os.environ))
    elif operation == "--cleanup-resume-terminal" and len(encoded) == 1:
        receipt = cleanup_resume_terminal(Path(project), native_id, json.loads(encoded[0]), dict(os.environ))
    else:
        raise RuntimeError("Invalid held terminal cleanup operation")
    print(json.dumps(receipt), flush=True)


def isolated_app_pid(application: Path) -> int | None:
    """Identify this invocation's extracted executable, never another app."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            if Path(os.readlink(entry / "exe")) == application:
                return int(entry.name)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return None


def isolated_window(clients: list[dict[str, object]], pid: int) -> dict[str, object] | None:
    matches = [client for client in clients if client.get("pid") == pid]
    if len(matches) > 1:
        raise RuntimeError("Isolated application has multiple compositor windows; refusing to resize")
    return matches[0] if matches else None


def size_isolated_window(application: Path, done: threading.Event,
                         state: dict[str, object]) -> None:
    """Hyprland tiles new windows; float only this package's verified PID/address."""
    try:
        deadline = time.monotonic() + 60
        pid = None
        while not done.is_set() and time.monotonic() < deadline:
            pid = isolated_app_pid(application)
            if pid is not None:
                state["pid"] = pid
                break
            done.wait(0.05)
        if pid is None or not os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"):
            return
        while not done.is_set() and time.monotonic() < deadline:
            clients = json.loads(subprocess.check_output(["hyprctl", "clients", "-j"], text=True))
            client = isolated_window(clients, pid)
            if client is None:
                done.wait(0.1)
                continue
            address = client.get("address")
            if not isinstance(address, str) or len(address) < 3 or not address.startswith("0x") or not all(
                    character in "0123456789abcdefABCDEF" for character in address[2:]):
                raise RuntimeError("Isolated window has an invalid compositor address")
            selector = "address:" + address
            if not client.get("floating"):
                subprocess.run(["hyprctl", "dispatch", "togglefloating", selector], check=True,
                               capture_output=True, text=True)
            subprocess.run(["hyprctl", "dispatch", "resizewindowpixel",
                            "exact 1280 800," + selector], check=True, capture_output=True, text=True)
            state["window"] = address
            return
        if not done.is_set():
            raise RuntimeError("No compositor window for the isolated packaged application")
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        state["error"] = error


def run_native_phase(application: Path, env: dict[str, str], phase: str) -> int:
    done = threading.Event()
    state: dict[str, object] = {}
    watcher = threading.Thread(target=size_isolated_window,
                               args=(application, done, state), daemon=True)
    watcher.start()
    try:
        run(["pnpm", "test:e2e:tauri:runner"], cwd=UI,
            env={**env, "CUTOKYO_NATIVE_PHASE": phase})
    finally:
        done.set()
        watcher.join()
    error = state.get("error")
    if isinstance(error, BaseException):
        raise RuntimeError("Could not size the isolated native app window") from error
    pid = state.get("pid")
    if not isinstance(pid, int):
        raise RuntimeError("Native runner never observed the packaged app process")
    if isolated_app_pid(application) is not None:
        raise RuntimeError("Packaged app survived the native runner; refusing process restart")
    return pid


def run_onboarding_worlds(build_root: Path, package: Path, evidence: Path,
                          build_env: dict[str, str]) -> None:
    """Additional package journeys never seed normalized desktop observations."""
    worlds = []
    for journey in ("browse", "capture-setup"):
        root = Path(tempfile.mkdtemp(prefix="cutokyo-native-test-wdio-"))
        print(f"Isolated native {journey} root: {root}", flush=True)
        application = root / "package/usr/bin/cutokyo-desktop"
        run(["dpkg-deb", "--extract", str(package), str(root / "package")], cwd=root, env=build_env)
        with reserve_port() as reservation:
            port = reservation.getsockname()[1]
            env = runtime_environment(root, evidence, application, port)
            env.pop("CUTOKYO_DESKTOP_TEST_FIXTURE")
            (root / "bin").mkdir(mode=0o700)
            shutil.copy2(build_root / "target/debug/cutokyo", root / "bin/cutokyo")
            if journey == "capture-setup":
                for name in ("claude", "codex", "opencode"):
                    shutil.copy2(build_root / "target/debug/fake-resume-harness", root / f"bin/{name}")
                for relative in ("home/.claude/settings.json", "home/.codex/hooks.json", "config/opencode/opencode.json"):
                    target = root / relative
                    target.parent.mkdir(parents=True, mode=0o700)
                    target.write_text('{\n  "user-only"  : "KEEP PACKAGE BYTE LAYOUT"\n}\n')
                    target.chmod(0o600)
            worlds.append({"journey": journey, "root": str(root), "application": str(application),
                           "application_sha256": sha256(application), "package_sha256": sha256(package),
                           "receiver_sha256": sha256(root / "bin/cutokyo"), "fixture_seeding": False})
            (evidence / "onboarding-package-worlds.json").write_text(json.dumps(worlds, indent=2))
            reservation.close()
            run_native_phase(application, env, journey)
            state = root / "data/desktop-state.json"
            shutil.copy2(state, evidence / f"{journey}-desktop-state.json")
            if journey == "capture-setup":
                prepare_native_hook_capture(root, env, evidence)
                run_native_phase(application, env, "capture-history")
                shutil.copy2(root / "resume-audit.txt", evidence / "native-hook-resume-audit.txt")
                shutil.copy2(root / "resume-audit.context.json", evidence / "native-hook-resume-audit.context.json")
        # A failure retains its entire world. Successful teardown has verified no app survived.
        shutil.rmtree(root)


def prepare_native_hook_capture(root: Path, env: dict[str, str], evidence: Path) -> None:
    """Execute the installed hook, then drain actual native-shaped spool evidence."""
    native_id = "12345678-1234-1234-1234-123456789abc"
    project = root / "project with spaces ' and $literal"
    project.mkdir(mode=0o700)
    transcript = root / f"home/.claude/projects/package/{native_id}.jsonl"
    transcript.parent.mkdir(parents=True, mode=0o700)
    transcript.write_text(json.dumps({"type": "assistant", "uuid": "message-package-20261004",
        "timestamp": "2026-10-04T04:00:00Z", "message": {"content": [
            {"type": "text", "text": "Packaged native transcript needle 20261004"}]}}) + "\n")
    transcript.chmod(0o600)
    settings = json.loads((root / "home/.claude/settings.json").read_text())
    argv = shlex.split(settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"])
    if argv[0] != str(root / "bin/cutokyo"):
        raise RuntimeError("Installed native hook did not bind the matching isolated CLI")
    payload = {"session_id": native_id, "cwd": str(project), "transcript_path": str(transcript),
               "hook_event_name": "UserPromptSubmit", "prompt_id": "prompt-package-20261004",
               "prompt": "Packaged hook search needle 20261004"}
    database = root / "data/cutokyo.db"
    before = sha256(database)
    capture = subprocess.run(argv, cwd=project, env=env, input=json.dumps(payload),
                             capture_output=True, text=True, timeout=10, check=True)
    if capture.stdout or capture.stderr or sha256(database) != before:
        raise RuntimeError("Native hook must publish silently without accessing the database")
    entries = list((root / "data/spool").glob("*.jsonl"))
    if len(entries) < 2:
        raise RuntimeError("Installed hook did not publish hook and verified transcript evidence")
    drained = subprocess.run([str(root / "bin/cutokyo"), "--config-file", str(root / "config/config.toml"),
                              "--data-dir", str(root / "data"), "drain", "--json"],
                             cwd=project, env=env, capture_output=True, text=True, timeout=15, check=True)
    receipt = json.loads(drained.stdout)
    if receipt["data"]["inserted"] < 2 or receipt["data"]["quarantined"]:
        raise RuntimeError("Actual native hook/transcript evidence did not drain cleanly")
    with (evidence / "native-hook-capture-drain.json").open("x") as output:
        json.dump({"native_id": native_id, "installed_hook_executed": True,
                   "database_unchanged_by_hook": True, "published_entries": len(entries),
                   "drain": receipt}, output, indent=2)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def interrupted(signum: int, _frame: object) -> None:
    raise InterruptedError(f"Native package runner interrupted by signal {signum}")


def main() -> None:
    signal.signal(signal.SIGTERM, interrupted)
    if sys.platform != "linux":
        raise RuntimeError("Native package journey currently requires Linux; no source-binary fallback")
    evidence_parent = REPOSITORY / "target/native-evidence"
    evidence_parent.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="attempt-", dir=evidence_parent))
    root = Path(tempfile.mkdtemp(prefix="cutokyo-native-test-wdio-"))
    print(f"Native package evidence: {evidence}", flush=True)
    print(f"Isolated native root: {root}", flush=True)
    # Preserve unsuccessful roots and artifacts. Successful roots are removed only
    # after run() has stopped and waited for its owned process group.
    env = build_environment(root)
    run(["cargo", "build", "--locked", "--manifest-path", str(REPOSITORY / "Cargo.toml"),
         "-p", "fake-harness", "--bin", "fake-resume-harness"], cwd=REPOSITORY, env=env)
    run(["cargo", "build", "--locked", "--manifest-path", str(REPOSITORY / "Cargo.toml"),
         "-p", "cutokyo-cli", "--bin", "cutokyo"], cwd=REPOSITORY, env=env)
    run(["cargo", "tauri", "build", "--debug", "--bundles", "deb", "--features", "native-e2e",
         "--config", json.dumps(tauri_config(root))], cwd=REPOSITORY / "crates/cutokyo-desktop", env=env)
    package, application = extract_package(root, env)
    retained_package = evidence / package.name
    shutil.copy2(package, retained_package)
    if sha256(retained_package) != sha256(package):
        raise RuntimeError("Retained native package differs from the tested artifact")
    (root / "bin").mkdir(mode=0o700)
    shutil.copy2(root / "target/debug/fake-resume-harness", root / "bin/claude")
    shutil.copy2(root / "target/debug/cutokyo", root / "bin/cutokyo")
    with reserve_port() as reservation:
        port = reservation.getsockname()[1]
        native_env = runtime_environment(root, evidence, application, port)
        manifest = {
            "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip(),
            "package": str(package), "package_sha256": sha256(package),
            "retained_package": str(retained_package),
            "application": str(application), "application_sha256": sha256(application),
            "receiver": str(root / "bin/cutokyo"),
            "receiver_sha256": sha256(root / "bin/cutokyo"),
            "synthetic_harness": str(root / "bin/claude"),
            "synthetic_harness_sha256": sha256(root / "bin/claude"),
            "asset_directory": str(root / "assets"), "root": str(root), "port": port,
            "launch_origin": "extracted-debian-bundle", "test_only": True,
        }
        with (evidence / "package-launch.json").open("x") as output:
            json.dump(manifest, output, indent=2)
        # Keep the port reserved throughout preparation; never use a fixed/shared
        # port. A subsequent bind collision must fail, not reuse another server.
        reservation.close()
        first_pid = run_native_phase(application, native_env, "save")
        second_pid = run_native_phase(application, native_env, "reopen")
        if second_pid == first_pid:
            raise RuntimeError("Native app was not restarted in a fresh process")
        with (evidence / "native-process-restart.json").open("x") as output:
            json.dump({"save_pid": first_pid, "reopen_pid": second_pid}, output)
    run_onboarding_worlds(root, package, evidence, env)
    shutil.copy2(root / "resume-audit.txt", evidence / "resume-audit.txt")
    shutil.copy2(root / "resume-audit.context.json", evidence / "resume-audit.context.json")
    shutil.rmtree(root)


if __name__ == "__main__":
    if sys.argv[1:2] in (["--inspect-resume-terminal"], ["--cleanup-resume-terminal"]):
        resume_cleanup_command(sys.argv[1:])
    else:
        main()

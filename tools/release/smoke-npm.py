#!/usr/bin/env python3
"""Install the generated npm wrapper and launch its packaged native executable."""

from __future__ import annotations

import argparse
import errno
import hashlib
import hmac
import json
import os
import re
import shutil
import socket
import signal
import subprocess
import sys
import tempfile
import threading
import tarfile
import zipfile
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import IO, ClassVar, NoReturn
from urllib.parse import quote


class ArtifactHandler(SimpleHTTPRequestHandler):
    """Serve one expected cargo-dist artifact and record every request."""

    expected_path: ClassVar[str] = ""
    request_log: ClassVar[list[str]] = []
    request_lock: ClassVar[threading.Lock] = threading.Lock()

    def log_message(self, format: str, *args: object) -> None:
        del format, args

    def do_GET(self) -> None:
        with self.request_lock:
            self.request_log.append(self.path)
        if self.path != self.expected_path:
            self.send_error(404, "unexpected artifact request")
            return
        super().do_GET()

    def do_HEAD(self) -> None:
        with self.request_lock:
            self.request_log.append(self.path)
        self.send_error(405, "only one GET is permitted")


def fail(message: str) -> NoReturn:
    print(f"smoke-npm: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stream_digest(stream: IO[bytes]) -> str:
    hasher = hashlib.sha256()
    while block := stream.read(1024 * 1024):
        hasher.update(block)
    return hasher.hexdigest()


def archived_executable_digest(archive: Path) -> str:
    name = "cutokyo.exe" if os.name == "nt" else "cutokyo"
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            members = [entry for entry in package.infolist() if Path(entry.filename).name == name and not entry.is_dir()]
            if len(members) != 1:
                fail("native archive must contain exactly one Cutokyo executable")
            with package.open(members[0]) as stream:
                return stream_digest(stream)
    with tarfile.open(archive, "r:*") as package:
        members = [entry for entry in package.getmembers() if Path(entry.name).name == name and entry.isfile()]
        if len(members) != 1:
            fail("native archive must contain exactly one regular Cutokyo executable")
        stream = package.extractfile(members[0])
        if stream is None:
            fail("native executable has no archive content")
        with stream:
            return stream_digest(stream)


def verify_native_checksum(checksum_path: Path, native_archive: Path) -> str:
    try:
        lines = [
            line.strip()
            for line in checksum_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
    except (OSError, UnicodeError) as error:
        fail(f"could not read native checksum: {error}")
    if len(lines) != 1:
        fail("native checksum must contain exactly one nonempty line")
    match = re.fullmatch(r"([0-9A-Fa-f]{64})\s+[ *]?([^/\\]+)", lines[0])
    if match is None:
        fail("native checksum is not a bounded SHA-256 sidecar")
    expected, filename = match.groups()
    if filename != native_archive.name:
        fail(
            "native checksum names a different artifact: "
            f"expected {native_archive.name}, received {filename}"
        )
    actual = sha256(native_archive)
    if not hmac.compare_digest(expected.lower(), actual):
        fail(
            "native archive checksum mismatch: "
            f"expected {expected.lower()}, received {actual}"
        )
    return actual


def run(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    identity: tuple[int, int] | None,
) -> subprocess.CompletedProcess[str]:
    executable = shutil.which(command[0], path=env.get("PATH")) or command[0]
    windows_batch = os.name == "nt" and Path(executable).suffix.lower() in {".cmd", ".bat"}
    invocation = subprocess.list2cmdline([executable, *command[1:]]) if windows_batch else command
    options = {}
    if identity is not None:
        options.update(user=identity[0], group=identity[1], extra_groups=[], umask=0o077)
    process = subprocess.Popen(invocation, cwd=cwd, env=env, shell=windows_batch,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                               start_new_session=True, **options)
    try:
        stdout, stderr = process.communicate(timeout=120)
    except BaseException:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], check=False,
                           capture_output=True, timeout=30)
        else:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        raise
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def least_privilege_identity() -> tuple[int, int] | None:
    if os.name == "nt" or not hasattr(os, "geteuid") or os.geteuid() != 0:
        return None
    try:
        import pwd

        account = pwd.getpwnam("nobody")
    except (ImportError, KeyError) as error:
        fail(f"refusing to run npm artifact commands as root: {error}")
    return account.pw_uid, account.pw_gid


def chown_tree(root: Path, identity: tuple[int, int] | None) -> None:
    if identity is None:
        return
    user, group = identity
    for directory, names, files in os.walk(root):
        os.chown(directory, user, group)
        for name in [*names, *files]:
            os.chown(Path(directory) / name, user, group, follow_symlinks=False)


def copy_input(source: Path, destination: Path) -> Path:
    target = destination / source.name
    shutil.copyfile(source, target)
    target.chmod(0o444)
    return target


def write_egress_guard(path: Path) -> None:
    path.write_text(
        """'use strict';
const net = require('node:net');
const tls = require('node:tls');
const dns = require('node:dns');

function hostOf(args) {
  const first = args[0];
  if (Array.isArray(first)) return hostOf(first);
  if (typeof first === 'object' && first !== null) {
    if (first.path) deny('unix-socket');
    return first.host || first.hostname || 'localhost';
  }
  if (typeof first === 'number' && typeof args[1] === 'string') return args[1];
  if (typeof first === 'string') deny('unix-socket');
  return 'localhost';
}
function isLoopback(host) {
  const value = String(host || '').toLowerCase().replace(/^\\[|\\]$/g, '');
  return value === 'localhost' || value === '127.0.0.1' || value === '::1';
}
function deny(host) {
  throw new Error(`CUTOKYO_EGRESS_DENIED:${String(host)}`);
}
function guardConnect(original) {
  return function guardedConnect(...args) {
    const host = hostOf(args);
    if (!isLoopback(host)) deny(host);
    return original.apply(this, args);
  };
}
net.Socket.prototype.connect = guardConnect(net.Socket.prototype.connect);
net.connect = guardConnect(net.connect);
net.createConnection = guardConnect(net.createConnection);
tls.connect = guardConnect(tls.connect);
const originalLookup = dns.lookup;
dns.lookup = function guardedLookup(host, ...args) {
  if (!isLoopback(host)) deny(host);
  return originalLookup.call(this, host, ...args);
};
""",
        encoding="utf-8",
    )


def scrubbed_environment(root: Path, npm_cache: Path, egress_guard: Path) -> dict[str, str]:
    allowed = (
        "COMSPEC",
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
    )
    environment = {key: os.environ[key] for key in allowed if key in os.environ}
    home = root / "home"
    temporary = root / "tmp"
    home.mkdir(mode=0o700)
    temporary.mkdir(mode=0o700)
    user_npmrc = home / "empty-user.npmrc"
    global_npmrc = home / "empty-global.npmrc"
    user_npmrc.write_text("", encoding="utf-8")
    global_npmrc.write_text("", encoding="utf-8")
    environment.update(
        {
            "HOME": os.fspath(home),
            "USERPROFILE": os.fspath(home),
            "TEMP": os.fspath(temporary),
            "TMP": os.fspath(temporary),
            "TMPDIR": os.fspath(temporary),
            "XDG_CONFIG_HOME": os.fspath(home / "config"),
            "XDG_DATA_HOME": os.fspath(home / "data"),
            "XDG_CACHE_HOME": os.fspath(home / "cache"),
            "APPDATA": os.fspath(home / "AppData" / "Roaming"),
            "LOCALAPPDATA": os.fspath(home / "AppData" / "Local"),
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "HTTP_PROXY": "http://127.0.0.1:9/",
            "HTTPS_PROXY": "http://127.0.0.1:9/",
            "ALL_PROXY": "http://127.0.0.1:9/",
            "http_proxy": "http://127.0.0.1:9/",
            "https_proxy": "http://127.0.0.1:9/",
            "all_proxy": "http://127.0.0.1:9/",
            "NODE_OPTIONS": f"--require={egress_guard}",
            "npm_config_audit": "false",
            "npm_config_cache": os.fspath(npm_cache),
            "npm_config_fund": "false",
            "npm_config_globalconfig": os.fspath(global_npmrc),
            "npm_config_offline": "true",
            "npm_config_registry": "http://127.0.0.1:9/",
            "npm_config_update_notifier": "false",
            "npm_config_userconfig": os.fspath(user_npmrc),
        }
    )
    return environment


def validate_egress_denial(
    root: Path,
    environment: dict[str, str],
    identity: tuple[int, int] | None,
) -> None:
    probe = run(
        [
            "node",
            "-e",
            "require('node:net').connect({host:'example.invalid',port:443})",
        ],
        cwd=root,
        env=environment,
        identity=identity,
    )
    if probe.returncode == 0 or "CUTOKYO_EGRESS_DENIED:example.invalid" not in probe.stderr:
        fail("the external-egress denial boundary did not reject the mutation probe")


def decode_json_output(completed: subprocess.CompletedProcess[str], command: str) -> dict[str, object]:
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        fail(f"installed {command} emitted invalid JSON: {error}")
    if not isinstance(payload, dict):
        fail(f"installed {command} JSON must be an object")
    return payload


def enter_linux_network_boundary(args: argparse.Namespace) -> int | None:
    if sys.platform != "linux":
        return None
    if Path(__file__) == Path("/smoke.py") and os.environ.get("CUTOKYO_NPM_SANDBOX") == "1":
        if {name for _, name in socket.if_nameindex()} != {"lo"}:
            fail("sandbox unexpectedly exposes a host network interface")
        # Probe the kernel boundary with native sockets, independently of Node
        # monkey-patching and proxy settings. TEST-NET-1 is never contacted.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.settimeout(2)
            try:
                probe.connect(("192.0.2.1", 443))
            except OSError as error:
                if error.errno not in {errno.ENETUNREACH, errno.EHOSTUNREACH}:
                    fail(f"native egress mutation did not prove routing denial: {error.errno}")
            else:
                fail("native egress unexpectedly escaped the loopback-only namespace")
        return None
    bubblewrap = shutil.which("bwrap")
    if bubblewrap is None:
        fail("environment-gap: Linux npm smoke requires bubblewrap for enforced egress denial")
    # Construct a new root, not a read-only view of the host root. In particular
    # no host home, /tmp, /run, IPC namespace or host process table is visible.
    node = subprocess.check_output(["node", "-p", "process.execPath"], text=True).strip()
    npm = shutil.which("npm")
    if npm is None:
        fail("environment-gap: npm runtime is missing")
    npm_root = Path(npm).resolve().parents[1]
    command = [bubblewrap, "--die-with-parent", "--unshare-all", "--new-session",
               "--clearenv", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
               "--dir", "/home", "--dir", "/run", "--dir", "/inputs",
               "--dir", "/runtime/bin", "--symlink", "usr/bin", "/bin",
               "--ro-bind", node, "/runtime/bin/node",
               "--ro-bind", str(npm_root), "/runtime/npm",
               "--symlink", "/runtime/npm/bin/npm-cli.js", "/runtime/bin/npm",
               "--ro-bind", str(Path(__file__).resolve()), "/smoke.py"]
    # Runtime libraries and interpreter standard libraries only. Executables are
    # individually allowlisted; user-controlled PATH directories are never mounted.
    import sysconfig
    architecture = sysconfig.get_config_var("MULTIARCH")
    if not architecture or not (Path("/usr/lib") / architecture).is_dir():
        fail("environment-gap: Linux sandbox requires a multiarch system runtime")
    for directory in (f"/usr/lib/{architecture}", sysconfig.get_path("stdlib"),
                      "/usr/lib64", "/lib", "/lib64"):
        path = Path(directory)
        if path.is_symlink():
            command.extend(["--symlink", os.readlink(path), directory])
        elif path.is_dir():
            command.extend(["--ro-bind", directory, directory])
    for name in ("env", "sh", "tar", "xz", "gzip", "uname", "getconf", "ldd"):
        executable = shutil.which(name, path="/usr/bin:/bin")
        if executable:
            command.extend(["--ro-bind", str(Path(executable).resolve()), f"/usr/bin/{name}"])
    python = Path(sys.executable).resolve()
    command.extend(["--ro-bind", str(python), str(python)])
    child_arguments = []
    for option in ("npm_package", "native_archive", "native_checksum", "dependency_package"):
        source = getattr(args, option).resolve()
        if not source.is_file():
            fail(f"required regular file is missing: {option.replace('_', ' ')}")
        destination = f"/inputs/{option}/{source.name}"
        command.extend(["--ro-bind", str(source), destination])
        child_arguments.extend(["--" + option.replace("_", "-"), destination])
    with tempfile.TemporaryDirectory(prefix="cutokyo-private-probe-") as private:
        sentinel = Path(private) / "synthetic-private-state"
        sentinel.write_text("SYNTHETIC-NOT-A-CREDENTIAL\n", encoding="utf-8")
        sentinel.chmod(0o600)
        # Supply only synthetic account data, even when the invoking builder is root.
        accounts = Path(private) / "passwd"
        accounts.write_text("nobody:x:65534:65534:nobody:/nonexistent:/usr/bin/sh\n", encoding="utf-8")
        command.extend(["--ro-bind", str(accounts), "/etc/passwd",
                        "--setenv", "PATH", "/runtime/bin:/usr/bin:/bin",
                        "--setenv", "HOME", "/tmp",
                        "--setenv", "CUTOKYO_NPM_SANDBOX", "1",
                        "--setenv", "CUTOKYO_SYNTHETIC_PRIVATE_PROBE", str(sentinel),
                        "--chdir", "/tmp", "--", str(python), "/smoke.py",
                        *child_arguments, "--expected-version", args.expected_version])
        completed = subprocess.run(command, check=False)
    return completed.returncode


def main() -> int:
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument("--npm-package", required=True, type=Path)
    parser.add_argument("--native-archive", required=True, type=Path)
    parser.add_argument("--native-checksum", required=True, type=Path)
    parser.add_argument("--dependency-package", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()
    isolated_exit = enter_linux_network_boundary(args)
    if isolated_exit is not None:
        return isolated_exit

    source_inputs = {
        "npm package": args.npm_package.resolve(),
        "native archive": args.native_archive.resolve(),
        "native checksum": args.native_checksum.resolve(),
        "npm dependency package": args.dependency_package.resolve(),
    }
    missing = [label for label, path in source_inputs.items() if not path.is_file()]
    if missing:
        fail("required regular files are missing: " + ", ".join(missing))
    native_digest = verify_native_checksum(
        source_inputs["native checksum"], source_inputs["native archive"]
    )
    input_digests = {label: sha256(path) for label, path in source_inputs.items()}
    executable_digest = archived_executable_digest(source_inputs["native archive"])

    with tempfile.TemporaryDirectory(prefix="cutokyo-npm-smoke-") as temporary_name:
        root = Path(temporary_name)
        root.chmod(0o700)
        inputs = root / "inputs"
        project = root / "consumer"
        npm_cache = root / "npm-cache"
        inputs.mkdir(mode=0o700)
        project.mkdir(mode=0o700)
        npm_cache.mkdir(mode=0o700)
        npm_package = copy_input(source_inputs["npm package"], inputs)
        native_archive = copy_input(source_inputs["native archive"], inputs)
        copy_input(source_inputs["native checksum"], inputs)
        dependency_package = copy_input(source_inputs["npm dependency package"], inputs)
        egress_guard = root / "deny-external-egress.cjs"
        write_egress_guard(egress_guard)
        (project / "package.json").write_text(
            '{"name":"cutokyo-artifact-smoke","private":true}\n', encoding="utf-8"
        )
        environment = scrubbed_environment(root, npm_cache, egress_guard)
        identity = least_privilege_identity()
        chown_tree(root, identity)
        validate_egress_denial(root, environment, identity)

        cached = run(
            ["npm", "cache", "add", os.fspath(dependency_package)],
            cwd=root,
            env=environment,
            identity=identity,
        )
        if cached.returncode != 0:
            fail(
                "could not seed the private offline npm cache: "
                f"{cached.stderr.strip()}"
            )
        install = run(
            [
                "npm",
                "install",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--offline",
                "--omit=dev",
                os.fspath(dependency_package),
                os.fspath(npm_package),
            ],
            cwd=project,
            env=environment,
            identity=identity,
        )
        if install.returncode != 0:
            fail(f"npm install exited {install.returncode}: {install.stderr.strip()}")

        installed = project / "node_modules" / "cutokyo"
        metadata_path = installed / "package.json"
        if not metadata_path.is_file():
            fail("npm did not install the cutokyo package")

        expected_request = "/" + quote(native_archive.name)
        request_log: list[str] = []
        request_lock = threading.Lock()
        ArtifactHandler.expected_path = expected_request
        ArtifactHandler.request_log = request_log
        ArtifactHandler.request_lock = request_lock
        handler = partial(
            ArtifactHandler,
            directory=os.fspath(native_archive.parent),
        )
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            # cargo-dist bakes release URLs into its generated package. Rebinding only
            # the installed test copy lets the real downloader consume a local dry-run
            # artifact without publishing or substituting a source-tree executable.
            metadata["artifactDownloadUrls"] = [
                f"http://127.0.0.1:{server.server_port}"
            ]
            metadata_path.write_text(
                json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            if sys.platform == "linux":
                sentinel = os.environ["CUTOKYO_SYNTHETIC_PRIVATE_PROBE"]
                script = installed / "install.js"
                contents = script.read_text(encoding="utf-8")
                # Execute the confidentiality regression in the actual installed
                # downloader, not an unrelated helper. Only this disposable copy
                # changes; the package/archive inputs remain byte-identical.
                probe = (
                    "let privateReadDenied = false;\n"
                    f"try {{ require('node:fs').readFileSync({json.dumps(sentinel)}); }} "
                    "catch (e) { if (['ENOENT','EACCES'].includes(e.code)) privateReadDenied = true; else throw e; }\n"
                    "if (!privateReadDenied) throw new Error('SYNTHETIC_PRIVATE_READ_ESCAPED');\n"
                    "console.error('CUTOKYO_SYNTHETIC_PRIVATE_READ_DENIED');\n"
                )
                if contents.startswith("#!"):
                    first, contents = contents.split("\n", 1)
                    probe = first + "\n" + probe
                script.write_text(probe + contents, encoding="utf-8")
            postinstall = run(
                ["node", "install.js"],
                cwd=installed,
                env=environment,
                identity=identity,
            )
            if sys.platform == "linux" and "CUTOKYO_SYNTHETIC_PRIVATE_READ_DENIED" not in postinstall.stderr:
                fail("installed downloader did not prove synthetic private-file read denial")
            if postinstall.returncode != 0:
                fail(
                    f"generated postinstall exited {postinstall.returncode}: "
                    f"{postinstall.stderr.strip()}"
                )
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        with request_lock:
            requests = list(request_log)
        if requests != [expected_request]:
            fail(
                "native artifact server expected exactly one request for "
                f"{expected_request!r}, received {requests!r}"
            )

        launcher = project / "node_modules" / ".bin" / (
            "cutokyo.cmd" if os.name == "nt" else "cutokyo"
        )
        if not launcher.exists():
            fail("npm package did not expose node_modules/.bin/cutokyo")
        environment["CUTOKYO_CONFIG_FILE"] = os.fspath(root / "config.toml")
        environment["CUTOKYO_DATA_DIR"] = os.fspath(root / "data")
        version = run(
            [os.fspath(launcher), "--json", "version"],
            cwd=project,
            env=environment,
            identity=identity,
        )
        if version.returncode != 0:
            fail(f"installed version exited {version.returncode}: {version.stderr.strip()}")
        version_payload = decode_json_output(version, "version")
        expected = args.expected_version.removeprefix("v")
        version_data = version_payload.get("data")
        actual = version_data.get("app_version") if isinstance(version_data, dict) else None
        if version_payload.get("ok") is not True or actual != expected:
            fail(f"expected installed version {expected!r}, received {actual!r}")

        doctor = run(
            [os.fspath(launcher), "--json", "doctor"],
            cwd=project,
            env=environment,
            identity=identity,
        )
        if doctor.returncode not in (0, 69, 78):
            fail(f"installed doctor exited {doctor.returncode}: {doctor.stderr.strip()}")
        doctor_payload = decode_json_output(doctor, "doctor")
        if doctor_payload.get("command") != "doctor":
            fail("installed doctor JSON omitted its command identity")

        native_name = "cutokyo.exe" if os.name == "nt" else "cutokyo"
        native_candidates = [
            path
            for path in installed.rglob(native_name)
            if path.is_file() and path.name == native_name and path.parent.name != ".bin"
        ]
        if len(native_candidates) != 1:
            fail(f"expected one installed native executable, found {len(native_candidates)}")
        native = native_candidates[0].resolve()
        try:
            native.relative_to(installed.resolve())
        except ValueError:
            fail("installed native executable resolves outside the npm package")

        if sha256(native) != executable_digest:
            fail("installed executable is not the exact cargo-dist archive binary")

        credential_names = [
            name
            for name in environment
            if re.search(
                r"(?:TOKEN|PASSWORD|AUTH|CREDENTIAL|CERTIFICATE|PRIVATE_KEY)",
                name,
                re.IGNORECASE,
            )
        ]
        print(
            json.dumps(
                {
                    "npm_package": source_inputs["npm package"].name,
                    "npm_sha256": input_digests["npm package"],
                    "native_archive": source_inputs["native archive"].name,
                    "native_checksum": source_inputs["native checksum"].name,
                    "native_sha256": native_digest,
                    "native_executable_sha256": executable_digest,
                    "dependency_package": source_inputs["npm dependency package"].name,
                    "dependency_sha256": input_digests["npm dependency package"],
                    "network_mode": "linux-network-namespace-loopback-only" if sys.platform == "linux" else "scrubbed-environment-loopback-only-node-boundary",
                    "native_egress_denied": sys.platform == "linux",
                    "external_egress_denied": True,
                    "artifact_request_count": len(requests),
                    "artifact_request_path": requests[0],
                    "credentials_in_child_environment": credential_names,
                    "temporary_home": True,
                    "least_privilege": sys.platform == "linux" and os.environ.get("CUTOKYO_NPM_SANDBOX") == "1",
                    "filesystem_mode": "allowlisted-runtime-and-inputs" if sys.platform == "linux" else "not-kernel-isolated",
                    "synthetic_private_read_denied": sys.platform == "linux",
                    "host_ipc_hidden": sys.platform == "linux",
                    "launcher": "node_modules/.bin/cutokyo",
                    "native_location": native.relative_to(project).as_posix(),
                    "source_tree_shortcut": False,
                    "version": actual,
                    "version_exit": version.returncode,
                    "doctor_exit": doctor.returncode,
                    "doctor_command": doctor_payload.get("command"),
                },
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

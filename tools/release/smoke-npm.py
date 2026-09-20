#!/usr/bin/env python3
"""Install the generated npm wrapper and launch its packaged native executable."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar, NoReturn
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
    if identity is None:
        return subprocess.run(
            command,
            cwd=cwd,
            env=env,
            check=False,
            capture_output=True,
            text=True,
            timeout=120,
            start_new_session=True,
        )
    user, group = identity
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        capture_output=True,
        text=True,
        timeout=120,
        start_new_session=True,
        user=user,
        group=group,
        extra_groups=[],
        umask=0o077,
    )


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
  if (typeof first === 'object' && first !== null) {
    return first.host || first.hostname || 'localhost';
  }
  if (typeof first === 'string' && args.length > 1) return args[1];
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


def main() -> int:
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument("--npm-package", required=True, type=Path)
    parser.add_argument("--native-archive", required=True, type=Path)
    parser.add_argument("--native-checksum", required=True, type=Path)
    parser.add_argument("--dependency-package", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()

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
            postinstall = run(
                ["node", "install.js"],
                cwd=installed,
                env=environment,
                identity=identity,
            )
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
                    "dependency_package": source_inputs["npm dependency package"].name,
                    "dependency_sha256": input_digests["npm dependency package"],
                    "network_mode": "scrubbed-environment-loopback-only-node-boundary",
                    "external_egress_denied": True,
                    "artifact_request_count": len(requests),
                    "artifact_request_path": requests[0],
                    "credentials_in_child_environment": credential_names,
                    "temporary_home": True,
                    "least_privilege": identity is not None or not hasattr(os, "geteuid") or os.geteuid() != 0,
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

#!/usr/bin/env python3
"""Build a production Debian artifact in the reviewed local builder, without launching it.

No harness state or credentials are mounted. The output directory must be new and
outside the worktree. Native/GUI acceptance is deliberately a separate serial step.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

IMAGE = "cutokyo-tauri-builder:2.11.4"


def output(command: list[str], **kwargs) -> str:
    return subprocess.check_output(command, text=True, **kwargs).strip()


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--pnpm-root", required=True, type=Path,
                        help="unpacked official pnpm 11.25.0 distribution, mounted read-only")
    parser.add_argument("--cargo-cache", required=True, type=Path,
                        help="Cargo dependency cache; only registry/ and git/ are mounted")
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    directory = args.output.resolve()
    if directory.is_relative_to(root) or directory.exists():
        parser.error("--output must be a new directory outside the worktree")
    if output(["git", "-C", str(root), "rev-parse", "--show-toplevel"]) != str(root):
        parser.error("--root must be the exact Git worktree")
    if output(["git", "-C", str(root), "diff", "HEAD", "--name-only"]):
        parser.error("production artifacts require clean, committed tracked source")
    revision = output(["git", "-C", str(root), "rev-parse", "HEAD"])
    image = json.loads(output(["docker", "image", "inspect", IMAGE]))[0]
    node = Path(output(["node", "-p", "process.execPath"])).resolve()
    node_root = node.parent.parent
    pnpm_root = args.pnpm_root.resolve(strict=True)
    cargo_cache = args.cargo_cache.resolve(strict=True)
    for subdirectory in ("git", "registry"):
        if not (cargo_cache / subdirectory).is_dir():
            parser.error(f"missing public Cargo dependency cache: {subdirectory}")
    for path in (root / "node_modules", root / "ui/node_modules", pnpm_root / "bin/pnpm.mjs"):
        if not path.exists():
            parser.error(f"required locked dependency installation is absent: {path}")
    pnpm_metadata = json.loads((pnpm_root / "package.json").read_text())
    if (pnpm_metadata.get("name"), pnpm_metadata.get("version")) != ("pnpm", "11.25.0"):
        parser.error("--pnpm-root must contain the official pnpm 11.25.0 distribution")
    # Vite bundles its config into this ignored cache even with dependencies read-only.
    (root / "ui/node_modules/.vite-temp").mkdir(exist_ok=True)
    directory.mkdir(mode=0o700, parents=True)
    volume = "cutokyo-c19-production-" + uuid.uuid4().hex
    container = volume + "-build"
    uid, gid = os.getuid(), os.getgid()
    command = [
        "docker", "run", "--rm", "--name", container, "--network", "none",
        "--user", f"{uid}:{gid}", "--workdir", str(root),
        "--env", "HOME=/tmp/cutokyo-build-home", "--env", "CARGO_HOME=/tmp/cutokyo-cargo",
        "--tmpfs", f"/tmp/cutokyo-cargo:rw,uid={uid},gid={gid},mode=0700",
        "--mount", f"type=bind,src={cargo_cache / 'git'},dst=/tmp/cutokyo-cargo/git,readonly",
        "--mount", f"type=bind,src={cargo_cache / 'registry'},dst=/tmp/cutokyo-cargo/registry,readonly",
        "--env", "CARGO_NET_OFFLINE=true", "--env", "CARGO_TARGET_DIR=/target",
        "--env", "RUSTUP_TOOLCHAIN=1.98.1",
        "--env", "PATH=/tmp/cutokyo-build-bin:/opt/node/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin",
        "--env", "COREPACK_ENABLE_NETWORK=0", "--env", "npm_config_manage_package_manager_versions=false",
        # pnpm records absolute workspace paths. Preserve them rather than
        # disabling verification or letting the offline build reinstall modules.
        "--env", "pnpm_config_verify_deps_before_run=error",
        "--mount", f"type=bind,src={root},dst={root}",
        "--mount", f"type=bind,src={root / 'node_modules'},dst={root / 'node_modules'},readonly",
        "--mount", f"type=bind,src={root / 'ui/node_modules'},dst={root / 'ui/node_modules'},readonly",
        "--tmpfs", f"{root / 'ui/node_modules/.vite-temp'}:rw,uid={uid},gid={gid},mode=0700",
        "--mount", f"type=bind,src={node_root},dst=/opt/node,readonly",
        "--mount", f"type=bind,src={pnpm_root},dst=/opt/pnpm,readonly",
        "--mount", f"type=volume,src={volume},dst=/target",
        image["Id"], "/bin/bash", "-euc",
        "mkdir -p /tmp/cutokyo-build-home /tmp/cutokyo-build-bin; "
        "ln -s /opt/pnpm/bin/pnpm.mjs /tmp/cutokyo-build-bin/pnpm; "
        "test \"$(cargo tauri --version)\" = 'tauri-cli 2.11.4'; "
        "test \"$(node --version)\" = 'v22.22.3'; "
        "test \"$(pnpm --version)\" = '11.25.0'; "
        "cargo tauri build --ci --features desktop-runtime --bundles deb",
    ]
    (directory / "build-inputs.json").write_text(json.dumps({
        "revision": revision, "tree": output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"]),
        "builder_image": IMAGE, "builder_image_id": image["Id"],
        "profile": "release", "features": ["desktop-runtime"],
        "recipe_sha256": digest(Path(__file__)), "command": command,
        "network": "none", "native_acceptance_performed": False,
    }, indent=2) + "\n")
    try:
        subprocess.run(["docker", "volume", "create", volume], check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["docker", "run", "--rm", "--network", "none", "--mount",
                        f"type=volume,src={volume},dst=/target", image["Id"],
                        "chown", f"{uid}:{gid}", "/target"], check=True)
        with (directory / "build.stdout.log").open("x") as stdout, (directory / "build.stderr.log").open("x") as stderr:
            completed = subprocess.run([
                sys.executable, str(root / "tools/scripts/source-snapshot.py"),
                "--root", str(root), "--manifest", str(directory / "source-immutability.json"),
                "--", *command,
            ], stdout=stdout, stderr=stderr, check=False)
        if completed.returncode:
            print(f"production build failed ({completed.returncode}); inspect {directory}", file=sys.stderr)
            return completed.returncode
        subprocess.run([
            "docker", "run", "--rm", "--network", "none", "--user", f"{uid}:{gid}",
            "--mount", f"type=volume,src={volume},dst=/target,readonly",
            "--mount", f"type=bind,src={directory},dst=/out", image["Id"], "/bin/bash", "-euc",
            "mapfile -d '' files < <(find /target/release/bundle/deb -maxdepth 1 -name '*.deb' -type f -print0); "
            "(( ${#files[@]} == 1 )); install -m 0644 \"${files[0]}\" /out/",
        ], check=True)
        packages = list(directory.glob("*.deb"))
        if len(packages) != 1:
            raise RuntimeError("builder did not emit exactly one production Debian package")
        package = packages[0]
        with tempfile.TemporaryDirectory(prefix="package-inspection-", dir=directory) as unpacked:
            subprocess.run(["dpkg-deb", "--extract", str(package), unpacked], check=True)
            binaries = list((Path(unpacked) / "usr/bin").iterdir())
            if len(binaries) != 1 or not binaries[0].is_file():
                raise RuntimeError("production package does not contain one native executable")
            binary_sha256 = digest(binaries[0])
        receipt = {
            "revision": revision, "profile": "release", "package": package.name,
            "sha256": digest(package), "binary_sha256": binary_sha256,
            "builder_image_id": image["Id"], "native_acceptance_performed": False,
            "source_immutability": "source-immutability.json",
        }
        (directory / "production-package.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt, sort_keys=True))
        return 0
    finally:
        subprocess.run(["docker", "rm", "--force", container], capture_output=True, check=False)
        subprocess.run(["docker", "volume", "rm", volume], check=False, stdout=subprocess.DEVNULL)



if __name__ == "__main__":
    raise SystemExit(main())

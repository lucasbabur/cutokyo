#!/usr/bin/env python3
"""Publish the tested CLI bytes without overwriting existing release assets."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.request

TARGETS = {
    "aarch64-apple-darwin": ".tar.xz",
    "x86_64-apple-darwin": ".tar.xz",
    "x86_64-unknown-linux-gnu": ".tar.xz",
    "x86_64-pc-windows-msvc": ".zip",
}


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def candidates(root: Path, tag: str, repository: str) -> list[Path]:
    if repository != "lucasbabur/cutokyo" or not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError("only stable tags from the official repository may publish")
    packages = list((root / "global-installers").glob("cutokyo-*.tgz"))
    if len(packages) != 1:
        raise ValueError("expected exactly one npm package")
    package = packages[0]
    with tarfile.open(package) as archive:
        stream = archive.extractfile("package/package.json")
        if stream is None:
            raise ValueError("npm metadata missing")
        metadata = json.load(stream)
    base = f"https://github.com/{repository}/releases/download/{tag}"
    if (metadata["name"] != "cutokyo" or metadata["version"] != tag[1:]
            or metadata["artifactDownloadUrls"] != [base]
            or metadata["repository"] != f"https://github.com/{repository}"):
        raise ValueError("npm identity/version/download URLs do not match the release")
    files = [package]
    for target, extension in TARGETS.items():
        name = f"cutokyo-cli-{target}{extension}"
        if metadata["supportedPlatforms"][target]["artifactName"] != name:
            raise ValueError(f"npm target does not match archive: {target}")
        native = root / f"cli-{target}" / name
        checksum = native.with_name(name + ".sha256")
        if checksum.read_text().split()[0] != digest(native):
            raise ValueError(f"native checksum mismatch: {name}")
        files.extend([native, checksum])
    return files


def gh(*args: str) -> str:
    return subprocess.check_output(["gh", *args], text=True)


def publish(root: Path, tag: str, repository: str) -> None:
    files = candidates(root, tag, repository)
    release = json.loads(gh("release", "view", tag, "--repo", repository,
                            "--json", "isDraft,tagName,assets"))
    if release["isDraft"] or release["tagName"] != tag:
        raise ValueError("an existing public release for the exact tag is required")
    existing = {asset["name"] for asset in release["assets"]}
    missing = []
    # Compare ALL collisions before uploading anything; never use --clobber.
    with tempfile.TemporaryDirectory(prefix="cutokyo-release-compare-") as directory:
        for path in files:
            if path.name not in existing:
                missing.append(path)
                continue
            gh("release", "download", tag, "--repo", repository, "--pattern", path.name,
               "--dir", directory)
            if digest(Path(directory) / path.name) != digest(path):
                raise ValueError(f"refusing to replace different published bytes: {path.name}")
        if missing:
            gh("release", "upload", tag, "--repo", repository, *map(str, missing))
    # Verify anonymous public downloads, as the npm postinstall has no GH_TOKEN.
    for path in files:
        url = f"https://github.com/{repository}/releases/download/{tag}/{path.name}"
        with urllib.request.urlopen(url, timeout=60) as response:
            actual = hashlib.file_digest(response, "sha256").hexdigest()
        if actual != digest(path):
            raise ValueError(f"public download differs from tested bytes: {path.name}")
    print(json.dumps({"tag": tag, "verified_public_assets": len(files)}))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    publish(args.artifacts, args.tag, args.repository)


if __name__ == "__main__":
    main()

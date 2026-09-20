#!/usr/bin/env python3
"""Create checksummed release inventory and a Tauri updater manifest."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn
from urllib.parse import quote

TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)$")
MAX_SIGNATURE_BYTES = 64 * 1024


def fail(message: str) -> NoReturn:
    print(f"create-manifest: {message}", file=sys.stderr)
    raise SystemExit(1)


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            hasher.update(block)
    return hasher.hexdigest()


def files_under(root: Path, excluded: set[Path]) -> list[Path]:
    return sorted(
        (
            path
            for path in root.rglob("*")
            if path.is_file() and path.resolve() not in excluded
        ),
        key=lambda path: path.relative_to(root).as_posix(),
    )


def source_date() -> str:
    completed = subprocess.run(
        ["git", "show", "-s", "--format=%cI", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode == 0:
        value = completed.stdout.strip()
        try:
            parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return parsed.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        except ValueError:
            pass
    return "1970-01-01T00:00:00Z"


def updater_platform(filename: str) -> str | None:
    lower = filename.lower()
    if lower.endswith(".appimage.tar.gz"):
        return "linux-aarch64" if any(token in lower for token in ("aarch64", "arm64")) else "linux-x86_64"
    if lower.endswith(".app.tar.gz"):
        return "darwin-aarch64" if any(token in lower for token in ("aarch64", "arm64")) else "darwin-x86_64"
    if lower.endswith((".msi.zip", ".nsis.zip")):
        return "windows-aarch64" if any(token in lower for token in ("aarch64", "arm64")) else "windows-x86_64"
    return None


def updater_entries(root: Path, repository: str, tag: str) -> dict[str, dict[str, str]]:
    result: dict[str, dict[str, str]] = {}
    signatures = sorted(root.rglob("*.sig"))
    all_files = {path.relative_to(root).as_posix(): path for path in root.rglob("*") if path.is_file()}
    for signature_path in signatures:
        relative_signature = signature_path.relative_to(root).as_posix()
        relative_payload = relative_signature.removesuffix(".sig")
        payload = all_files.get(relative_payload)
        if payload is None:
            fail(f"updater signature has no payload: {relative_signature}")
        platform = updater_platform(payload.name)
        if platform is None:
            continue
        if platform in result:
            fail(f"more than one updater payload was found for {platform}")
        if signature_path.stat().st_size > MAX_SIGNATURE_BYTES:
            fail(f"updater signature exceeds {MAX_SIGNATURE_BYTES} bytes: {relative_signature}")
        signature = signature_path.read_text(encoding="utf-8").strip()
        if not signature or any(character.isspace() for character in signature):
            fail(f"updater signature is empty or malformed: {relative_signature}")
        asset_name = quote(payload.name, safe="-._~")
        result[platform] = {
            "signature": signature,
            "url": f"https://github.com/{repository}/releases/download/{tag}/{asset_name}",
        }
    return result


def classify(path: Path) -> str:
    lower = path.name.lower()
    if lower.endswith(".sig"):
        return "updater-signature"
    if lower.endswith((".cdx.json", ".cdx.xml")):
        return "cyclonedx-sbom"
    if lower.endswith(".tgz"):
        return "npm-installer"
    if lower in {"install.sh", "install.ps1"} or lower.endswith(
        ("-installer.sh", "-installer.ps1")
    ):
        return "native-installer"
    if "/tauri-" in "/" + path.as_posix():
        return "desktop-package"
    if lower.endswith((".tar.gz", ".tar.xz", ".tar.zst", ".zip")):
        return "native-archive"
    return "release-metadata"


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--require-updater-signatures", action="store_true")
    args = parser.parse_args()

    tag_match = TAG.fullmatch(args.tag)
    if tag_match is None:
        fail("tag must be an explicit vMAJOR.MINOR.PATCH semantic version")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository):
        fail("repository must have the owner/name form")
    root = args.artifacts.resolve()
    if not root.is_dir():
        fail("artifact root does not exist")
    output = args.output.resolve()
    try:
        output.relative_to(root)
    except ValueError:
        fail("output must be inside the artifact root")

    latest_path = output.parent / "latest.json"
    entries = updater_entries(root, args.repository, args.tag)
    if args.require_updater_signatures:
        required_families = {"linux", "darwin", "windows"}
        actual_families = {platform.split("-", maxsplit=1)[0] for platform in entries}
        missing = sorted(required_families - actual_families)
        if missing:
            fail("signed updater payloads are missing for: " + ", ".join(missing))
    updater = {
        "version": tag_match.group("version"),
        "notes": "See the signed GitHub release notes.",
        "pub_date": source_date(),
        "platforms": entries,
    }
    atomic_json(latest_path, updater)

    excluded = {
        output,
        output.with_name(output.name + ".tmp"),
        root / "SHA256SUMS",
    }
    files = files_under(root, excluded)
    if not files:
        fail("artifact root is empty")
    by_asset_name: dict[str, list[str]] = {}
    for path in files:
        by_asset_name.setdefault(path.name, []).append(path.relative_to(root).as_posix())
    duplicates = {
        name: locations for name, locations in by_asset_name.items() if len(locations) > 1
    }
    if duplicates:
        detail = "; ".join(
            f"{name}: {', '.join(locations)}" for name, locations in sorted(duplicates.items())
        )
        fail(f"GitHub release asset names must be unique: {detail}")
    manifest_entries = []
    for path in files:
        relative = path.relative_to(root)
        manifest_entries.append(
            {
                "path": relative.as_posix(),
                "kind": classify(relative),
                "bytes": path.stat().st_size,
                "sha256": digest(path),
            }
        )
    manifest = {
        "schema_version": 1,
        "release": {
            "tag": args.tag,
            "version": tag_match.group("version"),
            "repository": args.repository,
        },
        "policy": {
            "native_cli_only": True,
            "npm_is_generated_installer": True,
            "checksums": "sha256",
            "sbom": "CycloneDX",
            "updater_signatures_required": args.require_updater_signatures,
        },
        "artifacts": manifest_entries,
    }
    atomic_json(output, manifest)
    print(json.dumps({"artifacts": len(files), "updater_platforms": sorted(entries)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

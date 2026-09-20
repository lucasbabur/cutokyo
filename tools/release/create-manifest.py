#!/usr/bin/env python3
"""Validate and inventory checksummed release artifacts and updater metadata."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import hmac
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn
from urllib.parse import quote

TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)$")
SHA256 = re.compile(r"^[0-9a-fA-F]{64}$")
MAX_CHECKSUM_BYTES = 16 * 1024 * 1024
MAX_SIGNATURE_BYTES = 64 * 1024
MAX_SMOKE_EVIDENCE_BYTES = 64 * 1024


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
    relative = "/" + path.as_posix().lower()
    if lower == "sha256sums" or lower == "sha256.sum" or lower.endswith("sha256sums") or lower.endswith(".sha256"):
        return "checksum"
    if lower.endswith(".sig"):
        return "updater-signature"
    if lower.endswith((".cdx.json", ".cdx.xml")):
        return "cyclonedx-sbom"
    if lower == "package-smoke.json":
        return "installed-package-smoke"
    if lower == "cargo-dist-plan.json":
        return "cargo-dist-plan"
    if lower == "latest.json":
        return "updater-manifest"
    if lower.endswith((".intoto.jsonl", ".intoto.json")):
        return "build-provenance"
    if lower.endswith(".tgz"):
        return "npm-installer"
    if lower in {"install.sh", "install.ps1"} or lower.endswith(("-installer.sh", "-installer.ps1")):
        return "native-installer"
    if "/tauri-" in relative:
        return "desktop-package"
    if lower.startswith("source.") and lower.endswith((".tar.gz", ".tar.xz", ".tar.zst", ".zip")):
        return "source-archive"
    if lower.endswith((".tar.gz", ".tar.xz", ".tar.zst", ".zip")):
        return "native-archive"
    return "release-metadata"


def parse_checksum_line(line: str, label: str) -> tuple[str, str]:
    parts = line.strip().split(maxsplit=1)
    if len(parts) != 2 or SHA256.fullmatch(parts[0]) is None:
        fail(f"{label} contains a malformed SHA-256 line")
    filename = parts[1].lstrip("*")
    if not filename:
        fail(f"{label} contains an empty artifact name")
    return parts[0].lower(), filename


def checksum_path(root: Path, raw_name: str, label: str) -> tuple[str, Path]:
    normalized = raw_name.removeprefix("./")
    relative = Path(normalized)
    if relative.is_absolute() or ".." in relative.parts or normalized in {"", "."}:
        fail(f"{label} contains an unsafe artifact path: {raw_name}")
    path = root / relative
    try:
        path.resolve().relative_to(root)
    except ValueError:
        fail(f"{label} resolves outside the artifact root: {raw_name}")
    return relative.as_posix(), path


def validate_complete_checksums(root: Path, excluded: set[Path]) -> Path:
    inventory = root / "SHA256SUMS"
    if not inventory.is_file() or inventory.is_symlink():
        fail("complete SHA256SUMS is missing or is not a regular file")
    if inventory.stat().st_size > MAX_CHECKSUM_BYTES:
        fail(f"SHA256SUMS exceeds {MAX_CHECKSUM_BYTES} bytes")
    entries: dict[str, str] = {}
    for line in inventory.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        expected, raw_name = parse_checksum_line(line, "SHA256SUMS")
        relative, path = checksum_path(root, raw_name, "SHA256SUMS")
        if relative in entries:
            fail(f"SHA256SUMS names an artifact more than once: {relative}")
        if not path.is_file() or path.is_symlink():
            fail(f"SHA256SUMS names a missing or unsafe artifact: {relative}")
        actual = digest(path)
        if not hmac.compare_digest(expected, actual):
            fail(
                f"checksum mismatch for {relative}: expected {expected}, received {actual}"
            )
        entries[relative] = expected
    expected_paths = {
        path.relative_to(root).as_posix()
        for path in files_under(root, excluded | {inventory.resolve()})
    }
    actual_paths = set(entries)
    missing = sorted(expected_paths - actual_paths)
    extra = sorted(actual_paths - expected_paths)
    if missing:
        fail("SHA256SUMS omits artifacts: " + ", ".join(missing))
    if extra:
        fail("SHA256SUMS names unexpected artifacts: " + ", ".join(extra))
    if not entries:
        fail("SHA256SUMS is empty")
    return inventory


def validate_archive_sidecars(files: list[Path], root: Path) -> None:
    by_relative = {path.relative_to(root).as_posix(): path for path in files}
    for payload in files:
        relative = payload.relative_to(root)
        if classify(relative) != "native-archive":
            continue
        sidecar_relative = relative.with_name(relative.name + ".sha256").as_posix()
        sidecar = by_relative.get(sidecar_relative)
        if sidecar is None:
            fail(f"native archive checksum sidecar is missing: {relative.as_posix()}")
        lines = [line for line in sidecar.read_text(encoding="utf-8").splitlines() if line.strip()]
        if len(lines) != 1:
            fail(f"native archive checksum sidecar is malformed: {sidecar_relative}")
        expected, raw_name = parse_checksum_line(lines[0], sidecar_relative)
        if raw_name != payload.name:
            fail(f"native archive checksum names a different payload: {sidecar_relative}")
        actual = digest(payload)
        if not hmac.compare_digest(expected, actual):
            fail(f"native archive checksum mismatch: {relative.as_posix()}")


def validate_sboms(files: list[Path], root: Path) -> None:
    sboms = [
        path for path in files if classify(path.relative_to(root)) == "cyclonedx-sbom"
    ]
    if not sboms:
        fail("required release artifact classes are missing: cyclonedx-sbom")
    for path in sboms:
        relative = path.relative_to(root).as_posix()
        if path.name.lower().endswith(".cdx.json"):
            try:
                value = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, json.JSONDecodeError) as error:
                fail(f"CycloneDX JSON SBOM is invalid ({relative}): {error}")
            if (
                not isinstance(value, dict)
                or value.get("bomFormat") != "CycloneDX"
                or not isinstance(value.get("specVersion"), str)
            ):
                fail(f"CycloneDX JSON SBOM contract is invalid: {relative}")
        elif path.stat().st_size == 0:
            fail(f"CycloneDX XML SBOM is empty: {relative}")


def validate_smoke_evidence(files: list[Path]) -> None:
    evidence_files = [path for path in files if path.name == "package-smoke.json"]
    if len(evidence_files) != 1:
        fail(f"expected exactly one installed-package smoke record, found {len(evidence_files)}")
    evidence_path = evidence_files[0]
    if evidence_path.stat().st_size > MAX_SMOKE_EVIDENCE_BYTES:
        fail(f"installed-package smoke record exceeds {MAX_SMOKE_EVIDENCE_BYTES} bytes")
    try:
        evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"installed-package smoke record is invalid: {error}")
    required = {
        "process_liveness": True,
        "product_readiness": False,
        "liveness_exit": 0,
        "product_readiness_exit": 69,
        "source_tree_shortcut": False,
    }
    mismatches = [
        field
        for field, expected in required.items()
        if not isinstance(evidence, dict) or evidence.get(field) != expected
    ]
    if mismatches:
        fail(
            "installed-package smoke does not distinguish liveness/readiness or package ownership: "
            + ", ".join(mismatches)
        )
    if not isinstance(evidence.get("package"), str) or not evidence["package"]:
        fail("installed-package smoke does not identify its package")


def validate_required_artifacts(files: list[Path], root: Path) -> None:
    kinds = {classify(path.relative_to(root)) for path in files}
    required = {
        "cargo-dist-plan",
        "cyclonedx-sbom",
        "desktop-package",
        "installed-package-smoke",
        "native-archive",
        "native-installer",
        "npm-installer",
    }
    missing = sorted(required - kinds)
    if missing:
        fail("required release artifact classes are missing: " + ", ".join(missing))


def validate_unique_asset_names(paths: list[Path], root: Path) -> None:
    by_asset_name: dict[str, list[str]] = {}
    for path in paths:
        by_asset_name.setdefault(path.name, []).append(path.relative_to(root).as_posix())
    duplicates = {
        name: locations for name, locations in by_asset_name.items() if len(locations) > 1
    }
    if duplicates:
        detail = "; ".join(
            f"{name}: {', '.join(locations)}" for name, locations in sorted(duplicates.items())
        )
        fail(f"GitHub release asset names must be unique: {detail}")


def atomic_bytes(path: Path, value: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(value)
    temporary.replace(path)


def atomic_json(path: Path, value: object) -> None:
    atomic_bytes(
        path,
        (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode(
            "utf-8"
        ),
    )


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
    generated = {
        output,
        output.with_name(output.name + ".tmp"),
        latest_path,
        latest_path.with_name(latest_path.name + ".tmp"),
    }
    checksum_inventory = validate_complete_checksums(root, generated)
    artifact_files = files_under(root, generated)
    validate_archive_sidecars(artifact_files, root)
    validate_sboms(artifact_files, root)
    validate_smoke_evidence(artifact_files)
    validate_required_artifacts(artifact_files, root)

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
    latest_path.parent.mkdir(parents=True, exist_ok=True)
    latest_bytes = (
        json.dumps(updater, indent=2, sort_keys=True, ensure_ascii=True) + "\n"
    ).encode("utf-8")

    paths_with_latest = artifact_files + [latest_path]
    validate_unique_asset_names(paths_with_latest, root)
    manifest_entries = [
        {
            "path": path.relative_to(root).as_posix(),
            "kind": classify(path.relative_to(root)),
            "bytes": path.stat().st_size,
            "sha256": digest(path),
        }
        for path in artifact_files
    ]
    manifest_entries.append(
        {
            "path": latest_path.relative_to(root).as_posix(),
            "kind": "updater-manifest",
            "bytes": len(latest_bytes),
            "sha256": hashlib.sha256(latest_bytes).hexdigest(),
        }
    )
    manifest_entries.sort(key=lambda entry: entry["path"])
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
            "checksums": "sha256-validated-before-manifest",
            "checksum_inventory": checksum_inventory.relative_to(root).as_posix(),
            "sbom": "CycloneDX-present-and-hashed",
            "updater_signatures_required": args.require_updater_signatures,
            "updater_platforms": sorted(entries),
            "probe_contract": {
                "process_liveness_exit": 0,
                "fresh_product_readiness_exit": 69,
                "same_signal": False,
            },
            "trust_boundaries": {
                "build_provenance": "created by a separate pinned GitHub attestation step; not asserted by this manifest",
                "notarization": "separate macOS tag-release gate; not asserted by this manifest",
                "platform_signing": "separate macOS and Windows tag-release gates; Linux is not platform signed",
                "updater_signing": "signature presence is inventoried separately from platform signing",
            },
        },
        "artifacts": manifest_entries,
    }
    atomic_bytes(latest_path, latest_bytes)
    atomic_json(output, manifest)
    print(json.dumps({"artifacts": len(manifest_entries), "updater_platforms": sorted(entries)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

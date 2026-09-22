#!/usr/bin/env python3
"""Validate, assemble, and inventory release artifacts and updater metadata."""

from __future__ import annotations

import argparse
import base64
import binascii
import datetime as dt
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NoReturn
from urllib.parse import quote

TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)$")
SHA256 = re.compile(r"^[0-9a-fA-F]{64}$")
MAX_CHECKSUM_BYTES = 16 * 1024 * 1024
MAX_SIGNATURE_BYTES = 64 * 1024
MAX_SMOKE_EVIDENCE_BYTES = 64 * 1024
UPDATER_PLATFORMS = {
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-aarch64",
    "linux-x86_64",
    "windows-aarch64",
    "windows-x86_64",
}


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
    files: list[Path] = []
    for path in root.rglob("*"):
        if path.is_symlink():
            fail(f"release artifacts may not be symbolic links: {path.relative_to(root)}")
        if path.is_file() and path.resolve() not in excluded:
            files.append(path)
    return sorted(files, key=lambda path: path.relative_to(root).as_posix())


def source_date() -> str:
    completed = subprocess.run(
        ["git", "show", "-s", "--format=%cI", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    if completed.returncode == 0:
        value = completed.stdout.strip()
        try:
            parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return parsed.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        except ValueError:
            pass
    return "1970-01-01T00:00:00Z"


def safe_relative(root: Path, raw_name: str, label: str) -> tuple[str, Path]:
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


def parse_updater_targets(root: Path, values: list[str]) -> dict[str, Path]:
    targets: dict[str, Path] = {}
    paths: set[str] = set()
    for value in values:
        platform, separator, raw_path = value.partition("=")
        if not separator or platform not in UPDATER_PLATFORMS:
            fail(
                "--updater-target must be PLATFORM=RELATIVE_PATH with a supported explicit platform"
            )
        relative, path = safe_relative(root, raw_path, "--updater-target")
        if platform in targets:
            fail(f"updater platform is mapped more than once: {platform}")
        if relative in paths:
            fail(f"updater payload is mapped more than once: {relative}")
        if not path.is_file() or path.is_symlink():
            fail(f"updater payload is missing or unsafe: {relative}")
        lower = path.name.lower()
        family = platform.split("-", maxsplit=1)[0]
        family_matches = {
            "linux": lower.endswith(".appimage"),
            "darwin": lower.endswith(".app.tar.gz"),
            "windows": lower.endswith((".msi", "-setup.exe")),
        }
        if not family_matches[family]:
            fail(f"updater payload suffix does not match {platform}: {relative}")
        targets[platform] = path
        paths.add(relative)
    return targets


def strict_tauri_base64(path: Path, label: str) -> str:
    if not path.is_file() or path.is_symlink() or path.stat().st_size > MAX_SIGNATURE_BYTES:
        fail(f"{label} must be a bounded regular file: {path}")
    try:
        encoded = path.read_text(encoding="utf-8").strip()
    except (OSError, UnicodeError) as error:
        fail(f"could not read {label}: {error}")
    if not encoded or any(character.isspace() for character in encoded):
        fail(f"{label} is not one canonical Base64 value: {path.name}")
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as error:
        fail(f"{label} has invalid Base64 ({path.name}): {error}")
    if base64.b64encode(decoded).decode("ascii") != encoded:
        fail(f"{label} Base64 is not canonical: {path.name}")
    try:
        decoded.decode("utf-8")
    except UnicodeDecodeError as error:
        fail(f"{label} does not contain UTF-8 Minisign text ({path.name}): {error}")
    return encoded


def verify_updater_signature(
    verifier: Path,
    public_key: Path,
    payload: Path,
    signature: Path,
) -> None:
    completed = subprocess.run(
        [
            os.fspath(verifier),
            "--payload",
            os.fspath(payload),
            "--signature",
            os.fspath(signature),
            "--public-key",
            os.fspath(public_key),
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
        env={},
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no verifier detail"
        fail(f"updater signature verification failed for {payload.name}: {detail}")


def updater_entries(
    root: Path,
    repository: str,
    tag: str,
    targets: dict[str, Path],
    require_signatures: bool,
    verifier: Path | None,
    public_key: Path | None,
) -> dict[str, dict[str, str]]:
    if require_signatures and (verifier is None or public_key is None):
        fail("required updater signatures need --signature-verifier and --updater-public-key")
    if verifier is not None and (not verifier.is_file() or verifier.is_symlink()):
        fail("signature verifier must be a regular executable file")
    if require_signatures and public_key is not None:
        strict_tauri_base64(public_key, "updater public key")

    # Tauri v2 also signs distribution packages (for example .deb) that are not
    # selected by latest.json. Verify every sidecar, not just mapped updater URLs.
    for signature_path in sorted(root.rglob("*.sig")):
        payload = signature_path.with_suffix("")
        if not payload.is_file() or payload.is_symlink():
            fail(f"updater signature has no regular payload: {signature_path.name}")
        strict_tauri_base64(signature_path, "updater signature")
        if verifier is None or public_key is None:
            fail("an updater signature was supplied without its verifier and public key")
        verify_updater_signature(verifier, public_key, payload, signature_path)

    result: dict[str, dict[str, str]] = {}
    for platform, payload in sorted(targets.items()):
        signature_path = payload.with_name(payload.name + ".sig")
        if not signature_path.is_file() or signature_path.is_symlink():
            if require_signatures:
                fail(f"signed updater payloads are missing for: {platform}")
            continue
        signature = strict_tauri_base64(signature_path, "updater signature")
        if verifier is None or public_key is None:
            fail("an updater signature was supplied without its verifier and public key")
        verify_updater_signature(verifier, public_key, payload, signature_path)
        asset_name = quote(payload.name, safe="-._~")
        result[platform] = {
            "signature": signature,
            "url": f"https://github.com/{repository}/releases/download/{tag}/{asset_name}",
        }
    if require_signatures:
        missing = {"linux", "darwin", "windows"} - {platform.split("-", 1)[0] for platform in result}
        if missing:
            fail("signed updater payloads are missing for: " + ", ".join(sorted(missing)))
    return result


def classify(path: Path) -> str:
    lower = path.name.lower()
    relative = "/" + path.as_posix().lower()
    if lower == "sha256.sum" or lower.endswith(("sha256sums", ".sha256")):
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
        relative, path = safe_relative(root, raw_name, "SHA256SUMS")
        if relative in entries:
            fail(f"SHA256SUMS names an artifact more than once: {relative}")
        if not path.is_file() or path.is_symlink():
            fail(f"SHA256SUMS names a missing or unsafe artifact: {relative}")
        actual = digest(path)
        if not hmac.compare_digest(expected, actual):
            fail(f"checksum mismatch for {relative}: expected {expected}, received {actual}")
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


def component_is_meaningful(value: object) -> bool:
    return (
        isinstance(value, dict)
        and value.get("type") in {"application", "library", "framework", "container", "file", "firmware"}
        and isinstance(value.get("name"), str)
        and bool(value["name"])
        and isinstance(value.get("version"), str)
        and bool(value["version"])
    )


def validate_sboms(files: list[Path], root: Path) -> None:
    sboms = [path for path in files if classify(path.relative_to(root)) == "cyclonedx-sbom"]
    if not sboms:
        fail("required release artifact classes are missing: cyclonedx-sbom")
    for path in sboms:
        relative = path.relative_to(root).as_posix()
        if path.name.lower().endswith(".cdx.json"):
            try:
                value = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, json.JSONDecodeError) as error:
                fail(f"CycloneDX JSON SBOM is invalid ({relative}): {error}")
            metadata = value.get("metadata") if isinstance(value, dict) else None
            product = metadata.get("component") if isinstance(metadata, dict) else None
            components = value.get("components") if isinstance(value, dict) else None
            if (
                not isinstance(value, dict)
                or value.get("bomFormat") != "CycloneDX"
                or not isinstance(value.get("specVersion"), str)
                or not isinstance(product, dict)
                or not component_is_meaningful(product)
                or not isinstance(components, list)
                or not components
                or not all(component_is_meaningful(component) for component in components)
            ):
                fail(f"CycloneDX JSON SBOM lacks a meaningful product or components: {relative}")
            if value["specVersion"] not in {"1.3", "1.4", "1.5", "1.6"} or type(value.get("version")) is not int or value["version"] < 1:
                fail(f"CycloneDX schema/version is unsupported: {relative}")
            references = [component.get("bom-ref") for component in [product, *components]]
            if not all(isinstance(reference, str) and reference for reference in references) or len(set(references)) != len(references):
                fail(f"CycloneDX components need unique bom-ref identities: {relative}")
            dependencies = value.get("dependencies")
            if not isinstance(dependencies, list) or not dependencies:
                fail(f"CycloneDX dependency graph is missing: {relative}")
            known = set(references)
            for dependency in dependencies:
                reference = dependency.get("ref") if isinstance(dependency, dict) else None
                children = dependency.get("dependsOn") if isinstance(dependency, dict) else None
                if (not isinstance(reference, str) or reference not in known
                    or not isinstance(children, list)
                    or any(not isinstance(ref, str) or ref not in known for ref in children)):
                    fail(f"CycloneDX dependency graph has an unknown reference: {relative}")
            if not any(edge["ref"] == product["bom-ref"] and edge["dependsOn"] for edge in dependencies):
                fail(f"CycloneDX dependency graph does not link the product: {relative}")
        else:
            fail(f"CycloneDX release validation requires JSON, not unchecked XML: {relative}")


def validate_provenance(files: list[Path], root: Path, repository: str, tag: str) -> None:
    provenance_files = [path for path in files if classify(path.relative_to(root)) == "build-provenance"]
    if not provenance_files:
        fail("required release artifact classes are missing: build-provenance")
    subjects_seen: set[str] = set()
    for path in provenance_files:
        relative = path.relative_to(root).as_posix()
        lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
        if not lines:
            fail(f"build provenance is empty: {relative}")
        for line in lines:
            try:
                statement = json.loads(line)
            except json.JSONDecodeError as error:
                fail(f"build provenance JSON is invalid ({relative}): {error}")
            subjects = statement.get("subject") if isinstance(statement, dict) else None
            if not isinstance(statement, dict) or statement.get("_type") != "https://in-toto.io/Statement/v1" or not isinstance(subjects, list) or not subjects:
                fail(f"build provenance has no in-toto subjects: {relative}")
            predicate = statement.get("predicate")
            if statement.get("predicateType") != "https://cutokyo.dev/attestation/release-inventory/v1" or not isinstance(predicate, dict):
                fail(f"build provenance predicate is missing or unsupported: {relative}")
            definition = predicate.get("buildDefinition", {})
            details = predicate.get("runDetails", {})
            if not isinstance(definition, dict) or not isinstance(details, dict):
                fail(f"build provenance definition/run details are invalid: {relative}")
            parameters = definition.get("externalParameters", {})
            if not isinstance(parameters, dict):
                fail(f"build provenance source identity is invalid: {relative}")
            revision = parameters.get("revision", "")
            if (parameters.get("repository") != repository or parameters.get("tag") != tag
                or not isinstance(revision, str) or re.fullmatch(r"[0-9a-f]{40}(?:[0-9a-f]{24})?", revision) is None
                or not isinstance(parameters.get("workflowRef"), str)
                or not parameters["workflowRef"].startswith(f"{repository}/.github/workflows/release.yml@")
                or not isinstance(definition.get("buildType"), str) or not definition["buildType"].startswith("https://")
                or definition.get("resolvedDependencies") != [{"uri": f"git+https://github.com/{repository}@{revision}", "digest": {"gitCommit": revision}}]
                or details.get("builder") != {"id": f"https://github.com/{repository}/actions"}
                or not isinstance(details.get("metadata"), dict)
                or not isinstance(details["metadata"].get("invocationId"), str)
                or not details["metadata"]["invocationId"].strip()):
                fail(f"build provenance source, builder or invocation identity is invalid: {relative}")
            for subject in subjects:
                name = subject.get("name") if isinstance(subject, dict) else None
                digests = subject.get("digest") if isinstance(subject, dict) else None
                expected = digests.get("sha256") if isinstance(digests, dict) else None
                if not isinstance(name, str) or not isinstance(expected, str) or SHA256.fullmatch(expected) is None:
                    fail(f"build provenance subject is malformed: {relative}")
                subject_relative, subject_path = safe_relative(root, name, relative)
                if subject_path.resolve() == path.resolve() or not subject_path.is_file() or subject_path.is_symlink():
                    fail(f"build provenance subject is missing or recursive: {subject_relative}")
                actual = digest(subject_path)
                if not hmac.compare_digest(expected.lower(), actual):
                    fail(f"build provenance subject digest mismatch: {subject_relative}")
                if subject_relative in subjects_seen:
                    fail(f"build provenance contains duplicate subjects: {subject_relative}")
                subjects_seen.add(subject_relative)
    required = {path.relative_to(root).as_posix() for path in files
                if classify(path.relative_to(root)) not in {"checksum", "build-provenance", "updater-signature"}}
    if required - subjects_seen:
        fail("build provenance omits release inputs: " + ", ".join(sorted(required - subjects_seen)))


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
        "first_uninstall_exit": 0,
        "repeat_uninstall_exit": 0,
        "no_state_restore_exits": [0, 0],
    }
    mismatches = [
        field
        for field, expected in required.items()
        if not isinstance(evidence, dict) or evidence.get(field) != expected
    ]
    if mismatches:
        fail(
            "installed-package smoke does not distinguish liveness/readiness, package ownership, or uninstall results: "
            + ", ".join(mismatches)
        )
    if not isinstance(evidence.get("package"), str) or not evidence["package"]:
        fail("installed-package smoke does not identify its package")
    packages = [path for path in files if path.name == evidence["package"] and path.suffix == ".deb"]
    if len(packages) != 1 or evidence.get("package_sha256") != digest(packages[0]):
        fail("installed-package smoke is not bound to the exact Debian artifact")
    executable_digest = evidence.get("installed_executable_sha256")
    if not isinstance(executable_digest, str) or SHA256.fullmatch(executable_digest) is None:
        fail("installed-package smoke omits its installed executable SHA-256")


def validate_required_artifacts(files: list[Path], root: Path) -> None:
    kinds = {classify(path.relative_to(root)) for path in files}
    required = {
        "build-provenance",
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
    duplicates = {name: locations for name, locations in by_asset_name.items() if len(locations) > 1}
    if duplicates:
        detail = "; ".join(
            f"{name}: {', '.join(locations)}" for name, locations in sorted(duplicates.items())
        )
        fail(f"GitHub release asset names must be unique: {detail}")


def atomic_bytes(path: Path, value: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            descriptor = -1
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode("utf-8")


def write_final_checksums(root: Path) -> Path:
    inventory = root / "SHA256SUMS"
    paths = files_under(root, {inventory.resolve()})
    value = "".join(f"{digest(path)} *{path.relative_to(root).as_posix()}\n" for path in paths)
    atomic_bytes(inventory, value.encode("utf-8"))
    return inventory


def validate_artifact_contract(files: list[Path], root: Path, repository: str, tag: str) -> None:
    validate_unique_asset_names(files, root)
    validate_archive_sidecars(files, root)
    validate_sboms(files, root)
    validate_smoke_evidence(files)
    validate_provenance(files, root, repository, tag)
    validate_required_artifacts(files, root)


def verify_manifest_inventory(root: Path, output: Path, latest_path: Path) -> None:
    try:
        manifest = json.loads(output.read_text(encoding="utf-8"))
        json.loads(latest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"generated release metadata is invalid: {error}")
    entries = manifest.get("artifacts") if isinstance(manifest, dict) else None
    if not isinstance(entries, list):
        fail("release manifest artifact inventory is invalid")
    recorded: set[str] = set()
    for entry in entries:
        relative = entry.get("path") if isinstance(entry, dict) else None
        expected = entry.get("sha256") if isinstance(entry, dict) else None
        size = entry.get("bytes") if isinstance(entry, dict) else None
        if not isinstance(relative, str) or not isinstance(expected, str) or not isinstance(size, int):
            fail("release manifest artifact entry is malformed")
        normalized, path = safe_relative(root, relative, "release manifest")
        if normalized in recorded or not path.is_file() or path.is_symlink():
            fail(f"release manifest artifact is duplicate or missing: {normalized}")
        if expected != digest(path) or size != path.stat().st_size:
            fail(f"release manifest artifact digest or size mismatch: {normalized}")
        recorded.add(normalized)
    expected_paths = {
        path.relative_to(root).as_posix()
        for path in files_under(root, {(root / "SHA256SUMS").resolve(), output.resolve()})
    }
    if recorded != expected_paths:
        missing = sorted(expected_paths - recorded)
        extra = sorted(recorded - expected_paths)
        fail(f"release manifest coverage mismatch; missing={missing}, extra={extra}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--require-updater-signatures", action="store_true")
    parser.add_argument("--updater-target", action="append", default=[])
    parser.add_argument("--signature-verifier", type=Path)
    parser.add_argument("--updater-public-key", type=Path)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()

    tag_match = TAG.fullmatch(args.tag)
    if tag_match is None:
        fail("tag must be an explicit vMAJOR.MINOR.PATCH semantic version")
    if re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository) is None:
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
    verifier = args.signature_verifier.resolve() if args.signature_verifier else None
    public_key = args.updater_public_key.resolve() if args.updater_public_key else None
    targets = parse_updater_targets(root, args.updater_target)

    if args.verify_only:
        validate_complete_checksums(root, set())
        artifact_files = files_under(
            root,
            {(root / "SHA256SUMS").resolve(), output.resolve(), latest_path.resolve()},
        )
        validate_artifact_contract(artifact_files, root, args.repository, args.tag)
        entries = updater_entries(
            root,
            args.repository,
            args.tag,
            targets,
            args.require_updater_signatures,
            verifier,
            public_key,
        )
        verify_manifest_inventory(root, output, latest_path)
        latest = json.loads(latest_path.read_text(encoding="utf-8"))
        manifest = json.loads(output.read_text(encoding="utf-8"))
        if (not isinstance(latest, dict) or latest.get("platforms") != entries
            or latest.get("version") != tag_match.group("version")
            or manifest.get("release") != {"tag": args.tag, "version": tag_match.group("version"), "repository": args.repository}
            or manifest.get("policy", {}).get("updater_signatures_required") is not args.require_updater_signatures
            or manifest.get("policy", {}).get("updater_platforms") != sorted(entries)):
            fail("generated updater/release metadata does not match the verified signing contract")
        print(json.dumps({"verified": True, "sha256_entries": len(files_under(root, {(root / 'SHA256SUMS').resolve()}))}))
        return 0

    generated = {
        output,
        latest_path,
    }
    validate_complete_checksums(root, {path.resolve() for path in generated})
    artifact_files = files_under(
        root,
        {path.resolve() for path in generated} | {(root / "SHA256SUMS").resolve()},
    )
    validate_artifact_contract(artifact_files, root, args.repository, args.tag)

    entries = updater_entries(
        root,
        args.repository,
        args.tag,
        targets,
        args.require_updater_signatures,
        verifier,
        public_key,
    )
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
    latest_bytes = json_bytes(updater)
    validate_unique_asset_names(artifact_files + [latest_path], root)
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
        "schema_version": 2,
        "release": {
            "tag": args.tag,
            "version": tag_match.group("version"),
            "repository": args.repository,
        },
        "policy": {
            "native_cli_only": True,
            "npm_is_generated_installer": True,
            "checksums": "final SHA256SUMS covers every final file except itself",
            "checksum_inventory": "SHA256SUMS",
            "checksum_circularity": "release-manifest.json does not embed SHA256SUMS; SHA256SUMS hashes the final manifest",
            "sbom": "CycloneDX product and components validated and hashed",
            "provenance": "in-toto subjects and SHA-256 digests validated; GitHub attestation remains a separate trust boundary",
            "updater_signatures_required": args.require_updater_signatures,
            "updater_platforms": sorted(entries),
            "updater_targets_explicit": True,
            "probe_contract": {
                "process_liveness_exit": 0,
                "fresh_product_readiness_exit": 69,
                "same_signal": False,
            },
            "trust_boundaries": {
                "build_provenance": "validated inventory statement; GitHub attestation is a separate pinned step",
                "notarization": "separate macOS tag-release gate; not asserted by this manifest",
                "platform_signing": "separate macOS and Windows tag-release gates; Linux is not platform signed",
                "updater_signing": "every mapped payload is cryptographically verified against the supplied reviewed public key",
            },
        },
        "artifacts": manifest_entries,
    }
    atomic_bytes(latest_path, latest_bytes)
    atomic_bytes(output, json_bytes(manifest))
    write_final_checksums(root)
    validate_complete_checksums(root, set())
    verify_manifest_inventory(root, output, latest_path)
    print(
        json.dumps(
            {
                "artifacts": len(manifest_entries),
                "updater_platforms": sorted(entries),
                "final_checksum_covers": ["latest.json", output.relative_to(root).as_posix()],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

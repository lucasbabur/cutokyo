#!/usr/bin/env python3
"""Create a deterministic in-toto inventory for assembled release inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path
from typing import NoReturn

REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
REVISION = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
TAG = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")
EXCLUDED_NAMES = {"SHA256SUMS", "latest.json", "release-manifest.json"}


def fail(message: str) -> NoReturn:
    print(f"create-provenance: {message}", file=sys.stderr)
    raise SystemExit(1)


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            hasher.update(block)
    return hasher.hexdigest()


def artifact_files(root: Path, output: Path) -> list[Path]:
    files: list[Path] = []
    for path in root.rglob("*"):
        if path.is_symlink():
            fail(f"release inputs may not be symbolic links: {path.relative_to(root)}")
        if path.is_file() and path.resolve() != output and path.name not in EXCLUDED_NAMES:
            files.append(path)
    files.sort(key=lambda path: path.relative_to(root).as_posix())
    if not files:
        fail("release input inventory is empty")
    return files


def atomic_json_line(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            descriptor = -1
            json.dump(value, stream, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--workflow-ref", required=True)
    parser.add_argument("--invocation-id", required=True)
    args = parser.parse_args()

    root = args.artifacts.resolve()
    output = args.output.resolve()
    if not root.is_dir():
        fail("artifact root does not exist")
    try:
        output.relative_to(root)
    except ValueError:
        fail("output must be inside the artifact root")
    if REPOSITORY.fullmatch(args.repository) is None:
        fail("repository must have the owner/name form")
    if REVISION.fullmatch(args.revision) is None:
        fail("revision must be a full lowercase Git object ID")
    if TAG.fullmatch(args.tag) is None:
        fail("tag must be an explicit semantic version")
    if not args.workflow_ref.strip() or not args.invocation_id.strip():
        fail("workflow reference and invocation ID must be nonempty")

    subjects = [
        {
            "name": path.relative_to(root).as_posix(),
            "digest": {"sha256": digest(path)},
        }
        for path in artifact_files(root, output)
    ]
    statement = {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": subjects,
        "predicateType": "https://cutokyo.dev/attestation/release-inventory/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": "https://github.com/lucasbabur/cutokyo/blob/main/docs/release/signing.md#cli-npm-checksums-and-sbom",
                "externalParameters": {
                    "repository": args.repository,
                    "revision": args.revision,
                    "tag": args.tag,
                    "workflowRef": args.workflow_ref,
                },
                "resolvedDependencies": [
                    {
                        "uri": f"git+https://github.com/{args.repository}@{args.revision}",
                        "digest": {"gitCommit": args.revision},
                    }
                ],
            },
            "runDetails": {
                "builder": {"id": f"https://github.com/{args.repository}/actions"},
                "metadata": {"invocationId": args.invocation_id},
            },
        },
    }
    atomic_json_line(output, statement)
    print(json.dumps({"subjects": len(subjects), "output": output.relative_to(root).as_posix()}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

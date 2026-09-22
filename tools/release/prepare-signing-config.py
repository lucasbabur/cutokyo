#!/usr/bin/env python3
"""Create the temporary Tauri v2 public signing overlay; never store private keys."""
from __future__ import annotations

import argparse
import base64
import binascii
import json
import os
from pathlib import Path
import re
import sys
import tempfile
from typing import NoReturn


def refuse(message: str) -> NoReturn:
    print(f"Release refused: {message}", file=sys.stderr)
    raise SystemExit(78)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--windows", action="store_true")
    args = parser.parse_args()
    if re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository) is None:
        refuse("repository must have owner/name form")
    key = os.environ.get("TAURI_UPDATER_PUBLIC_KEY", "").strip()
    try:
        decoded = base64.b64decode(key, validate=True)
        text = decoded.decode("utf-8")
    except (ValueError, UnicodeError, binascii.Error):
        refuse("TAURI_UPDATER_PUBLIC_KEY must be a canonical Tauri Base64 public key")
    if not key or base64.b64encode(decoded).decode("ascii") != key or not text.startswith("untrusted comment:"):
        refuse("TAURI_UPDATER_PUBLIC_KEY is absent or malformed")
    overlay = {
        "bundle": {"createUpdaterArtifacts": True},
        "plugins": {"updater": {"pubkey": key, "endpoints": [
            f"https://github.com/{args.repository}/releases/latest/download/latest.json"
        ]}},
    }
    if args.windows:
        thumbprint = os.environ.get("CUTOKYO_WINDOWS_CERTIFICATE_THUMBPRINT", "")
        if re.fullmatch(r"[0-9A-Fa-f]{40}", thumbprint) is None:
            refuse("the imported Windows Authenticode certificate thumbprint is absent or malformed")
        overlay["bundle"]["windows"] = {
            "certificateThumbprint": thumbprint, "digestAlgorithm": "sha256",
            "timestampUrl": "http://timestamp.digicert.com",
        }
    destination = args.output.absolute()
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".tauri-signing-", dir=destination.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(overlay, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, destination)  # Exclusive publication, never replace an existing config.
    finally:
        Path(temporary).unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

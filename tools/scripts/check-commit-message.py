#!/usr/bin/env python3
"""Require DCO and Claude attribution trailers in local commit messages."""

from __future__ import annotations

import pathlib
import re
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check-commit-message.py <message-file>", file=sys.stderr)
        return 2
    message = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
    signoff = re.search(r"(?mi)^Signed-off-by: .+ <[^>]+>$", message)
    coauthor = "Co-Authored-By: Claude Code <noreply@anthropic.com>" in message
    errors: list[str] = []
    if signoff is None:
        errors.append("missing Signed-off-by trailer; use git commit -s")
    if not coauthor:
        errors.append("missing required Claude Code co-author trailer")
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Reject mutable GitHub Action references in named workflows."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

USES_RE = re.compile(r"^\s*-\s+uses:\s*([^#\s]+)", re.MULTILINE)
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")


def check(path: Path) -> list[str]:
    failures: list[str] = []
    text = path.read_text(encoding="utf-8")
    for reference in USES_RE.findall(text):
        if reference.startswith("./"):
            continue
        if reference.startswith("docker://"):
            if not DIGEST_RE.fullmatch(reference):
                failures.append(
                    f"{path}: container action is not digest-pinned: {reference}"
                )
            continue
        if "@" not in reference:
            failures.append(f"{path}: action has no ref: {reference}")
            continue
        action, revision = reference.rsplit("@", 1)
        if not action or not SHA_RE.fullmatch(revision):
            failures.append(f"{path}: action is not pinned to a full SHA: {reference}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workflows", nargs="+", type=Path)
    args = parser.parse_args()
    failures = [
        failure for workflow in args.workflows for failure in check(workflow)
    ]
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"verified immutable action pins in {len(args.workflows)} workflow(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

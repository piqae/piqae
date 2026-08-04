#!/usr/bin/env python3
"""Classify changed repository paths for the bounded CI job set."""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable


GROUPS = (
    "rust",
    "web",
    "openapi",
    "terraform",
    "macos",
    "windows",
    "dependencies",
)


def classify(paths: Iterable[str], *, run_all: bool = False) -> dict[str, bool]:
    selected = {group: run_all for group in GROUPS}
    if run_all:
        return selected

    for raw_path in paths:
        path = raw_path.strip()
        if not path:
            continue

        workflow_change = path.startswith(".github/workflows/")
        rust_change = (
            path in {"Cargo.toml", "Cargo.lock"}
            or path.startswith((".cargo/", "bins/", "crates/", "migrations/", "xtask/"))
        )
        web_change = path in {
            "package.json",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
        } or path.startswith(("apps/", "contracts/", "deploy/cloudflare/", "sdk/"))

        selected["rust"] |= workflow_change or rust_change
        selected["web"] |= workflow_change or web_change
        selected["openapi"] |= workflow_change or path.startswith("contracts/openapi/")
        selected["terraform"] |= workflow_change or path.startswith("deploy/terraform/")
        selected["macos"] |= workflow_change or rust_change or path.startswith(
            ("packaging/macos/", "shells/macos/")
        )
        selected["windows"] |= workflow_change or rust_change or path.startswith(
            ("packaging/windows/", "shells/windows/")
        )
        selected["dependencies"] |= workflow_change or path in {
            "Cargo.lock",
            "Cargo.toml",
            "deny.toml",
            "release/security-exceptions.json",
        } or path.startswith((".cargo/", "bins/", "crates/", "xtask/")) and path.endswith(
            "Cargo.toml"
        )

    return selected


def input_paths(*, nul_delimited: bool) -> list[str]:
    data = sys.stdin.buffer.read()
    separator = b"\0" if nul_delimited else b"\n"
    return [item.decode("utf-8") for item in data.split(separator) if item]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--all", action="store_true", help="select every CI group")
    parser.add_argument(
        "--nul",
        action="store_true",
        help="read NUL-delimited paths from standard input",
    )
    args = parser.parse_args()
    selected = classify(input_paths(nul_delimited=args.nul), run_all=args.all)
    for group in GROUPS:
        print(f"{group}={'true' if selected[group] else 'false'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

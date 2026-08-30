#!/usr/bin/env python3
"""Fail-closed completion policy for the explicit all-platform release lane."""

from __future__ import annotations

import argparse
from dataclasses import dataclass


RESULTS = ("success", "failure", "cancelled", "skipped")


@dataclass(frozen=True)
class AggregateResults:
    core: str
    macos: str
    windows: str
    apple_sdk: str
    windows_sdk: str
    linux: str
    containers: str
    macos_promotion: str
    macos_prerelease: str
    container_promotion: str


def certification_errors(
    results: AggregateResults, *, windows_enabled: bool, publish: bool
) -> list[str]:
    expected = {
        "core": "success",
        "macos": "success",
        "windows": "success" if windows_enabled else "skipped",
        "apple-sdk": "success",
        "windows-sdk": "success",
        "linux": "success",
        "containers": "success",
        "macos-promotion": "success" if publish else "skipped",
        "macos-prerelease": "success" if publish else "skipped",
        "container-promotion": "success" if publish else "skipped",
    }
    actual = {
        "core": results.core,
        "macos": results.macos,
        "windows": results.windows,
        "apple-sdk": results.apple_sdk,
        "windows-sdk": results.windows_sdk,
        "linux": results.linux,
        "containers": results.containers,
        "macos-promotion": results.macos_promotion,
        "macos-prerelease": results.macos_prerelease,
        "container-promotion": results.container_promotion,
    }
    return [
        f"{lane} must be {required} for aggregate certification; got {actual[lane]}"
        for lane, required in expected.items()
        if actual[lane] != required
    ]


def _bool(value: str) -> bool:
    if value not in ("true", "false"):
        raise argparse.ArgumentTypeError("expected true or false")
    return value == "true"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--windows-enabled", type=_bool, required=True)
    parser.add_argument("--publish", type=_bool, required=True)
    for argument in (
        "core",
        "macos",
        "windows",
        "apple-sdk",
        "windows-sdk",
        "linux",
        "containers",
        "macos-promotion",
        "macos-prerelease",
        "container-promotion",
    ):
        parser.add_argument(f"--{argument}", choices=RESULTS, required=True)
    args = parser.parse_args()
    results = AggregateResults(
        core=args.core,
        macos=args.macos,
        windows=args.windows,
        apple_sdk=args.apple_sdk,
        windows_sdk=args.windows_sdk,
        linux=args.linux,
        containers=args.containers,
        macos_promotion=args.macos_promotion,
        macos_prerelease=args.macos_prerelease,
        container_promotion=args.container_promotion,
    )
    errors = certification_errors(
        results,
        windows_enabled=args.windows_enabled,
        publish=args.publish,
    )
    if errors:
        parser.exit(1, "\n".join(errors) + "\n")
    print("all effective selected release lanes passed aggregate certification")


if __name__ == "__main__":
    main()

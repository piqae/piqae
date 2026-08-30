#!/usr/bin/env python3
"""Classify the bounded artifact set for the canonical release workflow."""

from __future__ import annotations

import argparse
from dataclasses import dataclass


@dataclass(frozen=True)
class ReleasePlatform:
    name: str
    macos: bool
    windows: bool
    linux: bool
    containers: bool
    apple_sdk: bool
    windows_sdk: bool
    aggregate: bool


PLATFORMS = (
    "macos",
    "windows",
    "linux",
    "containers",
    "apple-sdk",
    "windows-sdk",
    "all",
)


def classify(value: str) -> ReleasePlatform:
    if value == "all":
        return ReleasePlatform(
            name=value,
            macos=True,
            windows=True,
            linux=True,
            containers=True,
            apple_sdk=True,
            windows_sdk=True,
            aggregate=True,
        )
    if value not in PLATFORMS:
        raise ValueError(f"release platform must be one of: {', '.join(PLATFORMS)}")
    return ReleasePlatform(
        name=value,
        macos=value == "macos",
        windows=value == "windows",
        linux=value == "linux",
        containers=value == "containers",
        apple_sdk=value == "apple-sdk",
        windows_sdk=value == "windows-sdk",
        aggregate=False,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("platform", choices=PLATFORMS)
    args = parser.parse_args()
    selection = classify(args.platform)
    print(f"platform={selection.name}")
    print(f"macos_enabled={str(selection.macos).lower()}")
    print(f"windows_selected={str(selection.windows).lower()}")
    print(f"linux_enabled={str(selection.linux).lower()}")
    print(f"containers_enabled={str(selection.containers).lower()}")
    print(f"apple_sdk_enabled={str(selection.apple_sdk).lower()}")
    print(f"windows_sdk_enabled={str(selection.windows_sdk).lower()}")
    print(f"aggregate_enabled={str(selection.aggregate).lower()}")


if __name__ == "__main__":
    main()

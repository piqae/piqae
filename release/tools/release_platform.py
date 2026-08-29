#!/usr/bin/env python3
"""Classify the bounded artifact set for the canonical release workflow."""

from __future__ import annotations

import argparse
from dataclasses import dataclass


@dataclass(frozen=True)
class ReleasePlatform:
    name: str
    macos: bool
    full: bool


def classify(value: str) -> ReleasePlatform:
    if value == "macos":
        return ReleasePlatform(name=value, macos=True, full=False)
    if value == "all":
        return ReleasePlatform(name=value, macos=True, full=True)
    raise ValueError("release platform must be macos or all")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("platform", choices=("macos", "all"))
    args = parser.parse_args()
    selection = classify(args.platform)
    print(f"platform={selection.name}")
    print(f"macos_enabled={str(selection.macos).lower()}")
    print(f"full_enabled={str(selection.full).lower()}")


if __name__ == "__main__":
    main()

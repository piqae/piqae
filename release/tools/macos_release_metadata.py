#!/usr/bin/env python3
"""Build and validate deterministic macOS release metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
STABLE = "https://downloads.piqae.com/releases/stable/"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(version: str, build: str, installer: Path, published_at: str) -> dict:
    file_name = f"piqae-macos-{version}-{build}-universal.pkg"
    return {
        "platform": "macos",
        "publishedAt": published_at,
        "artifact": {
            "id": "macos-universal",
            "platform": "macos",
            "title": "Piqae for macOS",
            "version": version,
            "fileName": file_name,
            "architectures": ["arm64", "x86_64"],
            "minimumOs": "macOS 13 or newer",
            "status": "preview",
            "statusReason": (
                "Signed and notarised coordinated updater; destructive rollback "
                "fault injection and physical-printer certification remain release gates."
            ),
            "downloadUrl": STABLE + file_name,
            "releaseUrl": "https://piqae.com/downloads",
            "sha256": sha256(installer),
            "checksumUrl": STABLE + file_name + ".sha256",
            "signing": {
                "status": "verified",
                "label": "Developer ID signed, Apple notarised and stapled",
            },
            "notes": [
                "Open the standard macOS Installer package for Apple silicon and Intel Macs.",
                "The menu app, local node, and CUPS executor are Developer ID signed.",
                (
                    "Sparkle replaces the menu application, which then verifies and "
                    "activates its matching durable agent and CUPS executor together "
                    "while the queue is idle."
                ),
                (
                    "The native component pair rolls back together if activation or "
                    "health verification fails; application rollback remains within "
                    "Sparkle's boundary."
                ),
            ],
        },
    }


def validate_appcast(path: Path, version: str, build: str) -> str:
    root = ET.parse(path).getroot()
    item = root.find("./channel/item")
    if item is None:
        raise ValueError("appcast has no release item")
    enclosure = item.find("enclosure")
    short = item.find(f"{{{SPARKLE}}}shortVersionString")
    bundle = item.find(f"{{{SPARKLE}}}version")
    if enclosure is None or short is None or bundle is None:
        raise ValueError("appcast is missing version or enclosure metadata")
    expected = STABLE + f"piqae-macos-{version}-{build}-update.zip"
    if short.text != version or bundle.text != build or enclosure.get("url") != expected:
        raise ValueError("appcast version, build, or download URL does not match")
    if not enclosure.get(f"{{{SPARKLE}}}edSignature"):
        raise ValueError("appcast enclosure is not signed")
    return expected


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    render = sub.add_parser("render")
    render.add_argument("--version", required=True)
    render.add_argument("--build", required=True)
    render.add_argument("--installer", type=Path, required=True)
    render.add_argument("--published-at", required=True)
    render.add_argument("--output", type=Path, required=True)
    check = sub.add_parser("validate-appcast")
    check.add_argument("--version", required=True)
    check.add_argument("--build", required=True)
    check.add_argument("--appcast", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "render":
            args.output.write_text(
                json.dumps(
                    artifact(args.version, args.build, args.installer, args.published_at),
                    indent=2,
                )
                + "\n"
            )
        else:
            print(validate_appcast(args.appcast, args.version, args.build))
    except (OSError, ValueError, ET.ParseError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Render idempotent, order-independent GitHub prerelease state notes."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


MARKER_START = "<!-- piqae-release-state\n"
MARKER_END = "\n-->"
PLATFORMS = ("macos", "windows")
PLATFORM_LABELS = {"macos": "macOS", "windows": "Windows"}
PLATFORM_STATES = ("pending", "published", "failed", "not-selected")
AGGREGATE_STATES = ("not-requested", "pending", "failed", "passed")


def initial_state(scope: str, *, windows_enabled: bool) -> dict[str, Any]:
    if scope == "all":
        return {
            "schemaVersion": 1,
            "scope": scope,
            "platforms": {
                "macos": "pending",
                "windows": "pending" if windows_enabled else "not-selected",
            },
            "aggregate": "pending",
        }
    if scope not in PLATFORMS:
        raise ValueError("prerelease notes require macos, windows, or all scope")
    return {
        "schemaVersion": 1,
        "scope": scope,
        "platforms": {scope: "pending"},
        "aggregate": "not-requested",
    }


def parse_state(
    notes: str, *, scope: str, windows_enabled: bool
) -> dict[str, Any]:
    start = notes.find(MARKER_START)
    if start == -1:
        return initial_state(scope, windows_enabled=windows_enabled)
    payload_start = start + len(MARKER_START)
    end = notes.find(MARKER_END, payload_start)
    if end == -1:
        raise ValueError("prerelease notes contain an unterminated Piqae state marker")
    raw = json.loads(notes[payload_start:end])
    if raw.get("schemaVersion") != 1 or raw.get("scope") != scope:
        raise ValueError("prerelease notes state identity does not match this release scope")
    platforms = raw.get("platforms")
    if not isinstance(platforms, dict) or any(
        platform not in PLATFORMS or status not in PLATFORM_STATES
        for platform, status in platforms.items()
    ):
        raise ValueError("prerelease notes contain invalid platform state")
    if raw.get("aggregate") not in AGGREGATE_STATES:
        raise ValueError("prerelease notes contain invalid aggregate state")
    return raw


def update_state(
    notes: str,
    *,
    scope: str,
    windows_enabled: bool,
    platform: str | None = None,
    aggregate: str | None = None,
) -> dict[str, Any]:
    state = parse_state(notes, scope=scope, windows_enabled=windows_enabled)
    if platform is not None:
        if platform not in state["platforms"]:
            raise ValueError(f"{platform} is not selected by the {scope} release scope")
        if state["platforms"][platform] == "not-selected":
            raise ValueError(f"{platform} is disabled by release policy")
        state["platforms"][platform] = "published"
    if aggregate is not None:
        if scope != "all" or aggregate not in ("failed", "passed"):
            raise ValueError("only all scope may finish aggregate certification")
        if aggregate == "passed" and any(
            status not in ("published", "not-selected")
            for status in state["platforms"].values()
        ):
            raise ValueError("aggregate certification cannot pass unpublished platforms")
        state["aggregate"] = aggregate
        if aggregate == "failed":
            for selected, status in state["platforms"].items():
                if status == "pending":
                    state["platforms"][selected] = "failed"
    return state


def render(state: dict[str, Any]) -> str:
    lines = [
        "# Piqae Preview release",
        "",
        "Platform publication:",
    ]
    descriptions = {
        "published": "published independently from verified platform evidence",
        "pending": "pending",
        "failed": "failed or did not complete",
        "not-selected": "not selected because the support tier is Disabled",
    }
    for platform in PLATFORMS:
        status = state["platforms"].get(platform)
        if status is not None:
            lines.append(f"- {PLATFORM_LABELS[platform]}: {descriptions[status]}.")
    aggregate = state["aggregate"]
    if aggregate == "not-requested":
        lines.extend(
            [
                "",
                "Aggregate all-platform certification was not requested for this release.",
            ]
        )
    else:
        aggregate_label = {
            "pending": "Pending",
            "failed": "Failed",
            "passed": "Passed",
        }[aggregate]
        lines.extend(
            [
                "",
                f"Aggregate all-platform certification: **{aggregate_label}**.",
            ]
        )
    lines.extend(
        [
            "",
            "Published platform artifacts remain Preview. No physical-print or Supported-platform claim is implied.",
            "",
            MARKER_START + json.dumps(state, sort_keys=True, separators=(",", ":")) + MARKER_END,
            "",
        ]
    )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--existing", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scope", choices=("macos", "windows", "all"), required=True)
    parser.add_argument("--windows-enabled", choices=("true", "false"), required=True)
    parser.add_argument("--platform", choices=PLATFORMS)
    parser.add_argument("--aggregate", choices=("failed", "passed"))
    args = parser.parse_args()
    notes = args.existing.read_text(encoding="utf-8")
    state = update_state(
        notes,
        scope=args.scope,
        windows_enabled=args.windows_enabled == "true",
        platform=args.platform,
        aggregate=args.aggregate,
    )
    args.output.write_text(render(state), encoding="utf-8")


if __name__ == "__main__":
    main()

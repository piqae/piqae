#!/usr/bin/env python3
"""Reject removed pre-release PrintPacket names from tracked source and artifacts."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

REMOVED_WIRE_IDENTIFIER = "piqae." + "business" + "-document/v1"
FORBIDDEN_TEXT = (
    "piqae." + "business" + "-document-pdf/v1",
    "Business" + "Document",
    "business" + "Documents",
    "business" + "_document",
    "business" + "-document",
    "piqae_" + "business" + "_documents",
    "/v1/" + "business" + "-document",
    "piqae.shopify-" + "business" + "-template/v1",
)
FORBIDDEN_MIGRATION_NAMES = (
    "business" + "_document",
    "document_adapter_" + "conversions",
)

# These locations assert rejection. The removed value may not appear in any
# producer, compatibility adapter, documentation, schema, or generated output.
NEGATIVE_FIXTURES = frozenset(
    {
        "apps/mcp/tests/server.test.ts",
        "apps/shopify/tests/template-model.test.ts",
        "crates/control-plane/src/api.rs",
        "crates/control-plane/src/documents.rs",
        "crates/document-renderer/src/lib.rs",
        "crates/printpacket/src/lib.rs",
        "crates/storage-postgres/tests/migrations.rs",
    }
)


def tracked_files(root: Path = ROOT) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
    )
    return [root / value.decode() for value in result.stdout.split(b"\0") if value]


def violations_for(path: str, content: str) -> list[str]:
    violations: list[str] = []
    if path.startswith("migrations/postgres/") and any(
        value in Path(path).name for value in FORBIDDEN_MIGRATION_NAMES
    ):
        violations.append(f"{path}: removed migration filename")

    for line_number, original in enumerate(content.splitlines(), 1):
        line = original
        if REMOVED_WIRE_IDENTIFIER in line:
            if path not in NEGATIVE_FIXTURES:
                violations.append(
                    f"{path}:{line_number}: removed wire identifier outside a negative fixture"
                )
            line = line.replace(REMOVED_WIRE_IDENTIFIER, "")
        for removed in FORBIDDEN_TEXT:
            if removed in line:
                violations.append(
                    f"{path}:{line_number}: removed PrintPacket predecessor name"
                )
    return violations


def repository_violations(root: Path = ROOT) -> list[str]:
    violations: list[str] = []
    for path in tracked_files(root):
        relative = path.relative_to(root).as_posix()
        try:
            content = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        violations.extend(violations_for(relative, content))
    return violations


def main() -> int:
    try:
        violations = repository_violations()
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"PrintPacket source policy could not inspect tracked files: {error}", file=sys.stderr)
        return 2
    if violations:
        print("PrintPacket source policy failed:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1
    print("PrintPacket source policy passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

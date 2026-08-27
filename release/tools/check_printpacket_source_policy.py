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

# The removed identifier is evidence only inside these exact rejection-test
# spans. A whole-file exception could accidentally admit a producer or adapter
# added elsewhere in a large source file.
NEGATIVE_FIXTURE_SPANS = {
    "apps/mcp/tests/server.test.ts": (
        'it("accepts only the canonical PrintPacket identifier"',
        "expect(fetchMock).not.toHaveBeenCalled();",
        1,
    ),
    "apps/shopify/tests/template-model.test.ts": (
        'it("rejects noncanonical packets and excessive nesting"',
        'toThrow("12 levels")',
        1,
    ),
    "crates/control-plane/src/api.rs": (
        "fn print_packet_capabilities_are_exact_bounded_and_printer_scoped()",
        "validate_document_render_capabilities(&retired_identifier).is_err()",
        1,
    ),
    "crates/control-plane/src/documents.rs": (
        "fn document_specs_are_bounded_and_reject_runtime_urls()",
        "experimental Piqae format identifier must not be normalized or migrated",
        1,
    ),
    "crates/document-renderer/src/lib.rs": (
        "fn rejects_old_format()",
        "Err(RenderError::UnsupportedVersion(_))",
        1,
    ),
    "crates/printpacket/src/lib.rs": (
        "fn canonical_document_is_analyzed_and_old_identifiers_are_rejected()",
        "Feature::MediaContinuous",
        2,
    ),
    "crates/storage-postgres/tests/migrations.rs": (
        "async fn documents_migrate_and_enforce_tenant_scoped_references()",
        "the pre-release format identifier must fail the database constraint",
        1,
    ),
}


def removed_identifier_is_structural_rejection(path: str, content: str) -> bool:
    fixture = NEGATIVE_FIXTURE_SPANS.get(path)
    if fixture is None:
        return False
    start_marker, end_marker, expected_count = fixture
    start = content.find(start_marker)
    end = content.find(end_marker, start + len(start_marker))
    if start < 0 or end < 0:
        return False
    end += len(end_marker)
    offsets: list[int] = []
    offset = content.find(REMOVED_WIRE_IDENTIFIER)
    while offset >= 0:
        offsets.append(offset)
        offset = content.find(REMOVED_WIRE_IDENTIFIER, offset + 1)
    return len(offsets) == expected_count and all(start <= value < end for value in offsets)


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
    structural_rejection = removed_identifier_is_structural_rejection(path, content)
    if path.startswith("migrations/postgres/") and any(
        value in Path(path).name for value in FORBIDDEN_MIGRATION_NAMES
    ):
        violations.append(f"{path}: removed migration filename")

    for line_number, original in enumerate(content.splitlines(), 1):
        line = original
        if REMOVED_WIRE_IDENTIFIER in line:
            if not structural_rejection:
                violations.append(
                    f"{path}:{line_number}: removed wire identifier outside an exact rejection test"
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

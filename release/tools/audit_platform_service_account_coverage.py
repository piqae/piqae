#!/usr/bin/env python3
"""Validate code-backed platform service-account release coverage."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

from check_platform_service_account_policy import (
    PolicyError,
    REQUIRED_SCENARIOS,
    load_json,
    support_matrix_tier,
)

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
STATUSES = {"missing", "partial", "passed"}


def repository_root() -> Path:
    return Path(__file__).resolve().parents[2]


def commit_is_ancestor(root: Path, commit: str) -> bool:
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", commit, "HEAD"],
        cwd=root,
        check=False,
        capture_output=True,
        timeout=10,
    )
    return result.returncode == 0


def validate_reference(root: Path, reference: dict[str, Any], scenario: str) -> None:
    path_value = reference.get("path")
    anchors = reference.get("anchors")
    if (
        not isinstance(path_value, str)
        or Path(path_value).is_absolute()
        or ".." in Path(path_value).parts
        or not isinstance(anchors, list)
        or not anchors
    ):
        raise PolicyError(f"{scenario}: invalid code reference")
    path = root / path_value
    if not path.is_file():
        raise PolicyError(f"{scenario}: referenced source is missing: {path_value}")
    content = path.read_text(encoding="utf-8")
    for anchor in anchors:
        if not isinstance(anchor, str) or not anchor or anchor not in content:
            raise PolicyError(
                f"{scenario}: source anchor is missing from {path_value}: {anchor!r}"
            )


def validate_coverage(
    coverage: dict[str, Any], *, root: Path, support_matrix: Path
) -> None:
    if coverage.get("schema_version") != 1:
        raise PolicyError("coverage schema_version must be 1")
    feature = coverage.get("feature")
    if feature != "platform_service_accounts":
        raise PolicyError("coverage feature must be platform_service_accounts")
    commit = coverage.get("implementation_commit")
    if not isinstance(commit, str) or not SHA_RE.fullmatch(commit):
        raise PolicyError("coverage implementation_commit must be a full Git SHA")
    if not commit_is_ancestor(root, commit):
        raise PolicyError("coverage implementation_commit is not an ancestor of HEAD")

    scenarios = coverage.get("scenarios")
    if not isinstance(scenarios, list):
        raise PolicyError("coverage scenarios must be an array")
    by_id = {
        scenario.get("id"): scenario
        for scenario in scenarios
        if isinstance(scenario, dict)
    }
    if set(by_id) != REQUIRED_SCENARIOS or len(by_id) != len(scenarios):
        raise PolicyError("coverage must contain every required scenario exactly once")

    for identifier, scenario in by_id.items():
        status = scenario.get("status")
        if status not in STATUSES:
            raise PolicyError(f"{identifier}: invalid coverage status {status!r}")
        references = scenario.get("code_references")
        commands = scenario.get("test_commands")
        missing = scenario.get("missing_evidence")
        if (
            not isinstance(references, list)
            or not isinstance(commands, list)
            or not isinstance(missing, list)
        ):
            raise PolicyError(f"{identifier}: coverage arrays are required")
        for reference in references:
            if not isinstance(reference, dict):
                raise PolicyError(f"{identifier}: invalid code reference")
            validate_reference(root, reference, identifier)
        for command in commands:
            if (
                not isinstance(command, str)
                or not command.startswith("cargo test ")
                or "\n" in command
                or "PIQAE_ALLOW_PHYSICAL_TESTS" in command
            ):
                raise PolicyError(f"{identifier}: unsafe test command")
        if any(not isinstance(item, str) or not item.strip() for item in missing):
            raise PolicyError(f"{identifier}: invalid missing-evidence entry")
        if status == "passed" and (not commands or missing):
            raise PolicyError(
                f"{identifier}: passed coverage requires tests and no missing evidence"
            )
        if status == "missing" and (references or commands):
            raise PolicyError(
                f"{identifier}: missing coverage cannot claim code or test evidence"
            )

    tier = support_matrix_tier(support_matrix, feature)
    if tier in {"preview", "supported"}:
        incomplete = sorted(
            identifier
            for identifier, scenario in by_id.items()
            if scenario["status"] != "passed"
        )
        if incomplete:
            raise PolicyError(
                f"{tier} tier has incomplete code coverage: {', '.join(incomplete)}"
            )


def main() -> int:
    root = repository_root()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--coverage",
        type=Path,
        default=root / "release" / "platform-service-account-coverage.json",
    )
    parser.add_argument(
        "--support-matrix",
        type=Path,
        default=root / "release" / "support-matrix.yaml",
    )
    args = parser.parse_args()
    try:
        validate_coverage(
            load_json(args.coverage), root=root, support_matrix=args.support_matrix
        )
    except (PolicyError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
        return 1
    print("platform service-account code coverage audit is truthful")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

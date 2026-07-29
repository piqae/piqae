#!/usr/bin/env python3
"""Fail-closed release policy for multi-workspace platform credentials."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path, PurePosixPath
from typing import Any

REQUIRED_SCENARIOS = {
    "tenant_isolation",
    "grant_revocation",
    "auditability",
    "ordinary_key_workspace_selection",
    "secret_redaction",
}
REQUIRED_EXTERNAL_EVIDENCE = {
    "independent_authorization_review",
    "production_audit_export_review",
    "credential_revocation_soak",
}
TIERS = {"disabled", "preview", "supported"}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")


class PolicyError(RuntimeError):
    """Platform credential evidence is incomplete or unsafe."""


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PolicyError(f"{path}: invalid JSON: {error}") from error
    if not isinstance(value, dict):
        raise PolicyError(f"{path}: root must be an object")
    return value


def support_matrix_tier(path: Path, feature: str) -> str:
    lines = path.read_text(encoding="utf-8").splitlines()
    in_features = False
    found_feature = False
    for line in lines:
        if line == "features:":
            in_features = True
            continue
        if in_features and line and not line.startswith(" "):
            break
        if in_features and line == f"  {feature}:":
            found_feature = True
            continue
        if found_feature:
            match = re.fullmatch(r"    tier: ([a-z]+)", line)
            if match:
                return match.group(1)
            if line.startswith("  ") and not line.startswith("    "):
                break
    raise PolicyError(f"{path}: feature {feature!r} has no tier")


def validate_policy(policy: dict[str, Any], support_matrix: Path) -> None:
    if policy.get("schema_version") != 1:
        raise PolicyError("policy schema_version must be 1")
    if policy.get("feature") != "platform_service_accounts":
        raise PolicyError("policy feature must be platform_service_accounts")
    tier = policy.get("current_tier")
    if tier not in TIERS:
        raise PolicyError(f"invalid current_tier: {tier!r}")
    matrix_tier = support_matrix_tier(support_matrix, policy["feature"])
    if tier != matrix_tier:
        raise PolicyError(
            f"policy tier {tier!r} does not match support matrix tier {matrix_tier!r}"
        )

    scenarios = policy.get("required_scenarios")
    if not isinstance(scenarios, list):
        raise PolicyError("required_scenarios must be an array")
    identifiers = [item.get("id") for item in scenarios if isinstance(item, dict)]
    if len(identifiers) != len(scenarios) or set(identifiers) != REQUIRED_SCENARIOS:
        raise PolicyError(
            "required_scenarios must contain exactly: "
            + ", ".join(sorted(REQUIRED_SCENARIOS))
        )
    if len(identifiers) != len(set(identifiers)):
        raise PolicyError("required_scenarios contains duplicate IDs")
    for scenario in scenarios:
        acceptance = scenario.get("acceptance")
        if (
            not isinstance(acceptance, list)
            or len(acceptance) < 2
            or any(not isinstance(item, str) or not item.strip() for item in acceptance)
        ):
            raise PolicyError(
                f"scenario {scenario['id']} requires at least two acceptance statements"
            )

    external = policy.get("supported_external_evidence")
    if not isinstance(external, list) or set(external) != REQUIRED_EXTERNAL_EVIDENCE:
        raise PolicyError(
            "supported_external_evidence must contain exactly: "
            + ", ".join(sorted(REQUIRED_EXTERNAL_EVIDENCE))
        )


def safe_reference(value: Any) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    path = PurePosixPath(value)
    return not path.is_absolute() and all(part not in {"", ".", ".."} for part in path.parts)


def validate_evidence(
    evidence: dict[str, Any], policy: dict[str, Any], *, now: dt.datetime
) -> None:
    if evidence.get("schema_version") != 1:
        raise PolicyError("evidence schema_version must be 1")
    if evidence.get("feature") != policy["feature"]:
        raise PolicyError("evidence feature does not match policy")
    if not SHA_RE.fullmatch(str(evidence.get("commit", ""))):
        raise PolicyError("evidence commit must be a full lowercase Git SHA")
    if not isinstance(evidence.get("release"), str) or not evidence["release"].strip():
        raise PolicyError("evidence release must be non-empty")
    try:
        recorded_at = dt.datetime.fromisoformat(
            str(evidence["recorded_at"]).removesuffix("Z") + "+00:00"
        )
    except (KeyError, ValueError) as error:
        raise PolicyError("evidence recorded_at must be an RFC 3339 UTC timestamp") from error
    if recorded_at.tzinfo != dt.timezone.utc or recorded_at > now:
        raise PolicyError("evidence recorded_at must be UTC and not in the future")

    results = evidence.get("results")
    if not isinstance(results, list):
        raise PolicyError("evidence results must be an array")
    by_id = {
        result.get("id"): result for result in results if isinstance(result, dict)
    }
    if set(by_id) != REQUIRED_SCENARIOS or len(results) != len(by_id):
        raise PolicyError("evidence must contain one result for every required scenario")
    for identifier, result in by_id.items():
        if result.get("status") != "passed":
            raise PolicyError(f"scenario {identifier} has not passed")
        command = result.get("command")
        if (
            not isinstance(command, str)
            or not command.startswith("cargo test ")
            or "SPOOL_ALLOW_PHYSICAL_TESTS" in command
            or "\n" in command
        ):
            raise PolicyError(f"scenario {identifier} has an unsafe test command")
        if not safe_reference(result.get("reference")):
            raise PolicyError(f"scenario {identifier} has an unsafe evidence reference")
        if result.get("synthetic_secrets_only") is not True:
            raise PolicyError(f"scenario {identifier} may contain reusable secrets")

    if policy["current_tier"] == "supported":
        reviews = evidence.get("external_evidence")
        if not isinstance(reviews, list):
            raise PolicyError("Supported tier requires external_evidence")
        by_id = {
            review.get("id"): review for review in reviews if isinstance(review, dict)
        }
        if set(by_id) != REQUIRED_EXTERNAL_EVIDENCE or len(reviews) != len(by_id):
            raise PolicyError("Supported tier requires every external evidence record")
        for identifier, review in by_id.items():
            if review.get("status") != "passed" or not safe_reference(
                review.get("reference")
            ):
                raise PolicyError(f"external evidence {identifier} is incomplete")


def check(policy_path: Path, matrix_path: Path, evidence_path: Path) -> None:
    policy = load_json(policy_path)
    validate_policy(policy, matrix_path)
    if policy["current_tier"] == "disabled":
        return
    if not evidence_path.is_file():
        raise PolicyError(
            f"{policy['current_tier']} tier requires evidence file {evidence_path}"
        )
    validate_evidence(evidence=load_json(evidence_path), policy=policy, now=dt.datetime.now(dt.timezone.utc))


def main() -> int:
    release = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--policy", type=Path, default=release / "platform-service-account-gates.json"
    )
    parser.add_argument(
        "--support-matrix", type=Path, default=release / "support-matrix.yaml"
    )
    parser.add_argument(
        "--evidence",
        type=Path,
        default=release / "evidence" / "platform-service-accounts.json",
    )
    args = parser.parse_args()
    try:
        check(args.policy, args.support_matrix, args.evidence)
    except PolicyError as error:
        print(error, file=sys.stderr)
        return 1
    print("platform service-account release policy is valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Validate that cargo-audit ignores are documented, bounded, and unexpired."""

from __future__ import annotations

import datetime as dt
import json
import sys
import tomllib
from pathlib import Path


def main() -> int:
    audit = tomllib.loads(Path(".cargo/audit.toml").read_text(encoding="utf-8"))
    ignored = set(audit.get("advisories", {}).get("ignore", []))
    policy = json.loads(
        Path("release/security-exceptions.json").read_text(encoding="utf-8")
    )
    if policy.get("schema_version") != 1:
        raise ValueError("security exception schema_version must be 1")
    exceptions = policy.get("exceptions")
    if not isinstance(exceptions, list):
        raise ValueError("security exceptions must be a list")
    documented: set[str] = set()
    today = dt.date.today()
    for exception in exceptions:
        required = {
            "id",
            "tool",
            "dependency",
            "scope",
            "owner",
            "review_by",
            "removal_condition",
        }
        if not isinstance(exception, dict) or not required.issubset(exception):
            raise ValueError("security exception is missing required fields")
        if exception["tool"] != "cargo-audit":
            raise ValueError(f"unsupported security exception tool: {exception['tool']}")
        identifier = exception["id"]
        if identifier in documented:
            raise ValueError(f"duplicate security exception: {identifier}")
        documented.add(identifier)
        review_by = dt.date.fromisoformat(exception["review_by"])
        if review_by < today:
            raise ValueError(
                f"security exception {identifier} expired on {review_by.isoformat()}"
            )
    if ignored != documented:
        raise ValueError(
            f"cargo-audit ignores and documented exceptions differ: "
            f"ignored={sorted(ignored)}, documented={sorted(documented)}"
        )
    print(f"validated {len(documented)} bounded security exception(s)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"security exception policy failed: {error}", file=sys.stderr)
        raise SystemExit(1)

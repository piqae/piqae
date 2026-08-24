#!/usr/bin/env python3
"""Require every workflow job to resolve its runner through a repository variable.

Runner capacity is an operational decision, not a source-code decision. Keeping
every `runs-on:` behind a `vars.PIQAE_*_RUNNER` indirection means the runner
fleet can be moved (for example onto larger third-party runners) by editing
repository variables alone, with no workflow edit, no review cycle, and an
instant revert by clearing the variable.

A job that hardcodes a runner label silently opts out of that control, so this
check fails closed on the first one.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

RUNS_ON_RE = re.compile(r"^(\s*)runs-on:[ \t]*(.*)$")
INDIRECTION_RE = re.compile(r"vars\.PIQAE_[A-Z0-9_]*_RUNNER")

# Protected pools are pinned to a physical fleet on purpose: the production
# promotion job must never be relocatable by editing a repository variable.
PROTECTED_POOL_RE = re.compile(r"^\[\s*self-hosted\b")


def check(path: Path) -> list[str]:
    failures: list[str] = []
    for number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        match = RUNS_ON_RE.match(line)
        if match is None:
            continue
        value = match.group(2).split("#", 1)[0].strip()
        if not value:
            failures.append(
                f"{path}:{number}: runs-on must be a single-line value so the "
                "runner indirection stays reviewable"
            )
            continue
        if PROTECTED_POOL_RE.match(value):
            continue
        if INDIRECTION_RE.search(value):
            continue
        failures.append(
            f"{path}:{number}: runner '{value}' is hardcoded; use "
            "${{ vars.PIQAE_<SCOPE>_RUNNER || '<default>' }} so the runner "
            "fleet stays switchable without a workflow edit"
        )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workflows", nargs="+", type=Path)
    args = parser.parse_args()
    failures = [failure for workflow in args.workflows for failure in check(workflow)]
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"verified runner indirection in {len(args.workflows)} workflow(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

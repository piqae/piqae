#!/usr/bin/env python3
"""Keep named competitor references inside the reviewed marketing surface."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
TERM = "print" + "node"
ALLOWED_FILES = {
    # Historical gitleaks fingerprints include immutable paths from public commits.
    ".gitleaksignore",
    "apps/web/src/routes/+page.svelte",
    "apps/web/src/routes/sitemap.xml/+server.ts",
    "apps/web/src/lib/components/marketing/ComparisonHero.svelte",
    "apps/web/src/lib/components/marketing/MarketingShell.svelte",
    "apps/web/src/lib/marketing/calculator.ts",
    "apps/web/src/lib/marketing/calculator.test.ts",
    "apps/web/src/lib/server/marketing-content.ts",
    "apps/web/tests/marketing.spec.ts",
}
ALLOWED_PREFIXES = (
    "apps/web/src/routes/compare/",
    "apps/web/src/routes/alternatives/",
    "apps/web/src/routes/migrate/",
    "apps/web/src/routes/tools/",
)


def is_marketing_path(path: str) -> bool:
    return path in ALLOWED_FILES or path.startswith(ALLOWED_PREFIXES)


def violations(paths: list[str]) -> list[str]:
    found: list[str] = []
    for relative in paths:
        if is_marketing_path(relative):
            continue
        if TERM in relative.lower():
            found.append(f"{relative}: competitor term appears in path")
            continue
        path = REPOSITORY_ROOT / relative
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (UnicodeDecodeError, OSError):
            continue
        found.extend(
            f"{relative}:{number}: competitor term appears outside marketing"
            for number, line in enumerate(lines, start=1)
            if TERM in line.lower()
        )
    return found


def tracked_paths() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
    )
    return [item.decode() for item in result.stdout.split(b"\0") if item]


def main() -> int:
    found = violations(tracked_paths())
    if found:
        print("Named competitor references must stay inside reviewed marketing files:")
        print("\n".join(f"- {item}" for item in found))
        return 1
    print("Competitor terminology boundary is clean.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

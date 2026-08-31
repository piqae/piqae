#!/usr/bin/env python3
"""Parse-check explicit Bash blocks embedded in GitHub Actions workflows."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import subprocess


SHELL_BASH = re.compile(r"^(?P<indent> *)shell:\s*bash\s*$")
RUN_BLOCK = re.compile(r"^(?P<indent> *)run:\s*\|[-+]?\s*$")
GITHUB_EXPRESSION = re.compile(r"\$\{\{.*?\}\}", re.DOTALL)


@dataclass(frozen=True)
class BashBlock:
    line: int
    script: str


def explicit_bash_blocks(path: Path) -> list[BashBlock]:
    lines = path.read_text(encoding="utf-8").splitlines()
    blocks: list[BashBlock] = []
    for shell_index, line in enumerate(lines):
        shell = SHELL_BASH.match(line)
        if shell is None:
            continue
        indent = len(shell.group("indent"))
        for run_index in range(shell_index + 1, len(lines)):
            candidate = lines[run_index]
            candidate_indent = len(candidate) - len(candidate.lstrip())
            if candidate.strip() and candidate_indent < indent:
                break
            run = RUN_BLOCK.match(candidate)
            if run is None or len(run.group("indent")) != indent:
                continue
            content_indent = indent + 2
            script_lines: list[str] = []
            for content in lines[run_index + 1 :]:
                content_leading = len(content) - len(content.lstrip())
                if content.strip() and content_leading < content_indent:
                    break
                script_lines.append(
                    content[content_indent:] if len(content) >= content_indent else ""
                )
            blocks.append(
                BashBlock(run_index + 1, "\n".join(script_lines).rstrip() + "\n")
            )
            break
    return blocks


def check(path: Path) -> list[str]:
    failures: list[str] = []
    for block in explicit_bash_blocks(path):
        # GitHub replaces expressions before invoking the configured shell.
        # Use one inert word so Bash sees the same surrounding quoting and
        # control-flow syntax without needing event-specific values.
        script = GITHUB_EXPRESSION.sub("github_expression", block.script)
        parsed = subprocess.run(
            ["bash", "-n"],
            input=script,
            text=True,
            capture_output=True,
            check=False,
        )
        if parsed.returncode != 0:
            detail = parsed.stderr.strip().replace("\n", " ")
            failures.append(f"{path}:{block.line}: invalid Bash block: {detail}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("workflows", nargs="+", type=Path)
    args = parser.parse_args()
    failures = [failure for workflow in args.workflows for failure in check(workflow)]
    if failures:
        print("\n".join(failures))
        return 1
    print(f"verified explicit Bash syntax in {len(args.workflows)} workflow(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

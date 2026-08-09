#!/usr/bin/env python3
"""Capture bounded, display-safe CUPS driver evidence without printing."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

QUEUE_RE = re.compile(r"^[A-Za-z0-9_.-]{1,127}$")
MAX_OUTPUT = 4 * 1024 * 1024
MAX_FEATURES = 512
MAX_CHOICES = 512
PPD_FIELDS = {"Manufacturer", "ModelName", "NickName", "Product", "PSVersion"}


def bounded_text(value: str) -> str:
    value = value.strip().strip('"')
    if len(value) > 512 or any(ord(char) < 32 and char not in "\t" for char in value):
        raise ValueError("driver evidence contains an unsafe or oversized value")
    return value


def parse_choices(encoded: str) -> list[dict[str, object]]:
    choices: list[dict[str, object]] = []
    for token in encoded.split():
        selected = token.startswith("*")
        token = token.removeprefix("*")
        if "/" not in token and choices:
            continue  # continuation of a display label, not a new native value
        value = bounded_text(token.split("/", 1)[0])
        if value and not any(choice["value"] == value for choice in choices):
            choices.append({"value": value, "default": selected})
        if len(choices) > MAX_CHOICES:
            raise ValueError("driver advertised more than 512 choices for one feature")
    return choices


def parse_lpoptions(output: str) -> list[dict[str, object]]:
    if len(output.encode("utf-8")) > MAX_OUTPUT:
        raise ValueError("lpoptions output exceeded 4 MiB")
    features: list[dict[str, object]] = []
    for raw_line in output.splitlines():
        if ":" not in raw_line:
            continue
        heading, encoded = raw_line.split(":", 1)
        key = bounded_text(heading.split("/", 1)[0])
        choices = parse_choices(encoded)
        if key and choices:
            features.append({"key": key, "choices": choices})
        if len(features) > MAX_FEATURES:
            raise ValueError("driver advertised more than 512 features")
    return features


def ppd_evidence(path: Path | None) -> tuple[dict[str, str], dict[str, object] | None]:
    if path is None or not path.is_file():
        return {}, None
    data = path.read_bytes()
    if len(data) > MAX_OUTPUT:
        raise ValueError("PPD exceeded 4 MiB")
    metadata: dict[str, str] = {}
    for raw_line in data.decode("utf-8", errors="replace").splitlines():
        if not raw_line.startswith("*") or ":" not in raw_line:
            continue
        key, value = raw_line[1:].split(":", 1)
        if key in PPD_FIELDS:
            metadata[key] = bounded_text(value)
    digest = hashlib.sha256(data).hexdigest()
    canonical_name = "driver.ppd"
    inventory = f"{canonical_name}\0{len(data)}\0{digest}".encode()
    return metadata, {
        "canonical_inventory_sha256": hashlib.sha256(inventory).hexdigest(),
        "files": [{"name": canonical_name, "size_bytes": len(data), "sha256": digest}],
    }


def redact_queue(value: object, printer: str) -> object:
    if isinstance(value, dict):
        return {key: redact_queue(item, printer) for key, item in value.items()}
    if isinstance(value, list):
        return [redact_queue(item, printer) for item in value]
    if isinstance(value, str):
        return value.replace(printer, "[redacted-queue]")
    return value


def capture(printer: str, lpoptions_file: Path | None, ppd_file: Path | None) -> dict[str, object]:
    if not QUEUE_RE.fullmatch(printer):
        raise ValueError("printer must be a CUPS queue name using letters, numbers, dot, dash or underscore")
    if lpoptions_file:
        output = lpoptions_file.read_text(encoding="utf-8")
    else:
        result = subprocess.run(
            ["/usr/bin/lpoptions", "-p", printer, "-l"],
            check=True,
            capture_output=True,
            text=True,
            timeout=15,
        )
        output = result.stdout
    effective_ppd = ppd_file
    if effective_ppd is None:
        candidate = Path("/etc/cups/ppd") / f"{printer}.ppd"
        effective_ppd = candidate if candidate.is_file() else None
    metadata, package = ppd_evidence(effective_ppd)
    evidence = {
        "schema_version": 1,
        "source": "cups.lpoptions",
        "redacted": True,
        "printer": {
            "platform": "cups_ipp",
            "manufacturer": metadata.get("Manufacturer"),
            "driver_name": metadata.get("NickName") or metadata.get("ModelName"),
            "driver_version": metadata.get("PSVersion"),
        },
        "driver_package": package,
        "capabilities": parse_lpoptions(output),
    }
    return redact_queue(evidence, printer)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--printer", required=True, help="local CUPS queue name; never written to output")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--lpoptions-file", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--ppd-file", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    evidence = capture(args.printer, args.lpoptions_file, args.ppd_file)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Wrote redacted, non-printing driver evidence to {args.output}")


if __name__ == "__main__":
    main()

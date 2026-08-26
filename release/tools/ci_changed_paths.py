#!/usr/bin/env python3
"""Classify changed paths into dependency-aware CI scopes."""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable

GROUPS = (
    "rust_server",
    "rust_shared",
    "macos_rust",
    "macos_shell",
    "macos_packaging",
    "windows_rust",
    "windows_shell",
    "windows_installer",
    "web",
    "sdk",
    "mcp",
    "shopify",
    "openapi",
    "terraform",
    "release_tooling",
    "dependency_policy",
)

NATIVE_SHARED = (
    "crates/agent-client/",
    "crates/agent-core/",
    "crates/agent-storage/",
    "crates/domain/",
    "crates/executor-protocol/",
    "crates/executor-supervisor/",
    "crates/local-api/",
    "crates/local-ipc/",
    "crates/node-client/",
    "crates/node-ffi/",
    "crates/node-host-api/",
    "crates/node-runtime/",
    "crates/piqae-agent/",
    "crates/protocol/",
    "crates/update-guardian/",
    "crates/update-metadata/",
)


def classify(paths: Iterable[str], *, run_all: bool = False) -> dict[str, bool]:
    selected = {group: run_all for group in GROUPS}
    if run_all:
        return selected

    for raw_path in paths:
        path = raw_path.strip()
        if not path:
            continue
        if path in {
            ".github/workflows/ci.yml",
            "release/tools/ci_changed_paths.py",
            "release/tools/test_ci_changed_paths.py",
        }:
            for group in GROUPS:
                selected[group] = True
            continue
        root_rust = path in {"Cargo.toml", "Cargo.lock"} or path.startswith(".cargo/")
        shared = root_rust or path.startswith(NATIVE_SHARED)
        # Every workspace crate must at least run Linux Rust CI. Keep an explicit
        # list only for the narrower native-platform fan-out; otherwise adding a
        # crate can silently create an untested path.
        server = shared or path.startswith(("crates/", "migrations/", "bins/", "xtask/"))
        js_workspace = path in {"package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml"}
        openapi = path.startswith("contracts/openapi/")
        sdk = path.startswith("sdk/")

        selected["rust_shared"] |= shared
        selected["rust_server"] |= server
        selected["macos_rust"] |= shared or path.startswith("crates/executor-cups/")
        selected["windows_rust"] |= shared or path.startswith(
            (
                "crates/executor-windows/",
                "crates/shell-windows/",
                "sdk/dotnet/",
                "sdk/native/",
            )
        )
        selected["macos_shell"] |= path.startswith(
            ("shells/macos/", "sdk/apple/")
        ) or path == "release/tools/test_apple_node_sdk.sh"
        selected["macos_packaging"] |= path.startswith("packaging/macos/")
        selected["windows_shell"] |= path.startswith("crates/shell-windows/")
        selected["windows_installer"] |= path.startswith("packaging/windows/")
        selected["web"] |= js_workspace or openapi or path.startswith(
            ("apps/web/", "contracts/", "deploy/cloudflare/")
        )
        selected["sdk"] |= js_workspace or openapi or sdk
        # MCP and Shopify consume @piqae/sdk as a workspace dependency, so SDK
        # and contract changes must test those downstream consumers too.
        selected["mcp"] |= js_workspace or openapi or sdk or path.startswith("apps/mcp/")
        selected["shopify"] |= (
            js_workspace or openapi or sdk or path.startswith("apps/shopify/")
        )
        selected["openapi"] |= openapi
        selected["terraform"] |= path.startswith("deploy/terraform/")
        selected["dependency_policy"] |= root_rust or path in {
            "deny.toml",
            "release/security-exceptions.json",
        } or (path.endswith("Cargo.toml") and path.startswith(("bins/", "crates/", "xtask/")))

        if path.startswith(".github/workflows/"):
            selected["release_tooling"] = True
            name = path.removeprefix(".github/workflows/")
            selected["sdk"] |= name in {"sdk-release.yml", "release.yml"}
            selected["mcp"] |= name == "mcp-release.yml"
            selected["shopify"] |= name == "shopify-deploy.yml"
            selected["dependency_policy"] |= name == "supply-chain.yml"
            selected["macos_packaging"] |= name in {
                "macos-release.yml",
                "macos-promotion.yml",
                "recover-macos-release.yml",
                "release.yml",
            }
            selected["windows_installer"] |= name in {"windows-release.yml", "release.yml"}
        if path.startswith(("packaging/release/", "release/tools/")):
            selected["release_tooling"] = True

    return selected


def input_paths(*, nul_delimited: bool) -> list[str]:
    data = sys.stdin.buffer.read()
    separator = b"\0" if nul_delimited else b"\n"
    return [item.decode("utf-8") for item in data.split(separator) if item]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--nul", action="store_true")
    args = parser.parse_args()
    for group, enabled in classify(
        input_paths(nul_delimited=args.nul), run_all=args.all
    ).items():
        print(f"{group}={'true' if enabled else 'false'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

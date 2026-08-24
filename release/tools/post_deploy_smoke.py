#!/usr/bin/env python3
"""Assert that a deployment serves its real surface at the reviewed revision.

A liveness probe answers `200` from any build. That is enough to conclude the
process started and not enough to conclude the reviewed change is live or that
its endpoints work in this deployment's configuration.

This gate makes two separate claims:

1. **Identity.** The origin reports the expected service and the expected
   commit. A stale container, a rolled-back revision, or a deploy that silently
   landed a different commit fails here.
2. **Surface.** Real endpoints answer as themselves. Endpoints that require
   authentication must answer `401`/`403`; a `5xx` means the route exists in the
   binary but is structurally unavailable in this deployment's configuration,
   which is exactly the failure a single-configuration test suite cannot see.
   A `2xx` from an unauthenticated tenant endpoint is treated as a failure too,
   because that would be an authentication bypass.

Limits, stated honestly: the probe list in `release/post-deploy-probes.json` is
representative, not exhaustive, and no probe is authenticated, so this proves
routes are reachable and configured, not that their responses are correct.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Callable

DEFAULT_MANIFEST = Path(__file__).resolve().parents[1] / "post-deploy-probes.json"
REVISION_RE = re.compile(r"^[0-9a-f]{40}$")
# Status families a probe may legitimately answer with.
PUBLIC_OK = (200,)
AUTHENTICATED_OK = (401, 403)


class SmokeError(Exception):
    """A deployment did not meet the post-deploy contract."""


def load_manifest(path: Path) -> dict:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema") != 1:
        raise SmokeError(f"{path} is not a schema 1 probe manifest")
    return manifest


def service_manifest(manifest: dict, service: str) -> dict:
    services = manifest.get("services", {})
    if service not in services:
        known = ", ".join(sorted(services)) or "none"
        raise SmokeError(f"no probes are declared for '{service}'; known: {known}")
    return services[service]


def check_health(payload: object, service: str, revision: str | None) -> list[str]:
    """Validates a health document without performing any I/O."""
    failures: list[str] = []
    if not isinstance(payload, dict):
        return [f"health response is not a JSON object: {payload!r}"]
    if payload.get("status") != "ok":
        failures.append(f"health status is {payload.get('status')!r}, expected 'ok'")
    if payload.get("service") != service:
        failures.append(
            f"health service is {payload.get('service')!r}, expected {service!r}"
        )
    if revision is not None:
        reported = payload.get("revision")
        if reported != revision:
            failures.append(
                f"health revision is {reported!r}, expected {revision!r}; the "
                "origin is not serving the reviewed commit"
            )
    return failures


def evaluate(path: str, expect: str, status: int) -> str | None:
    """Returns a failure description for a probe result, or `None` when it passed."""
    if status >= 500:
        return (
            f"{path} answered {status}: the route is deployed but unavailable in "
            "this deployment's configuration"
        )
    if expect == "public":
        if status not in PUBLIC_OK:
            return f"{path} answered {status}, expected {PUBLIC_OK[0]}"
        return None
    if expect == "authenticated":
        if 200 <= status < 300:
            return (
                f"{path} answered {status} without credentials, which would be an "
                "authentication bypass"
            )
        if status not in AUTHENTICATED_OK:
            return f"{path} answered {status}, expected one of {AUTHENTICATED_OK}"
        return None
    return f"{path} declares an unknown expectation {expect!r}"


def fetch(url: str, timeout: float) -> tuple[int, bytes]:
    request = urllib.request.Request(url, method="GET", headers={"accept": "*/*"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, response.read(1_000_000)
    except urllib.error.HTTPError as error:
        return error.code, error.read(1_000_000)
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise SmokeError(f"{url} is unreachable: {error}") from error


def await_revision(
    origin: str,
    health_path: str,
    service: str,
    revision: str | None,
    attempts: int,
    interval: float,
    timeout: float,
    fetcher: Callable[[str, float], tuple[int, bytes]],
    sleeper: Callable[[float], None],
) -> None:
    url = f"{origin}{health_path}"
    failures = ["the deployment was never probed"]
    for attempt in range(attempts):
        try:
            status, body = fetcher(url, timeout)
            if status != 200:
                failures = [f"health answered {status}"]
            else:
                failures = check_health(json.loads(body.decode("utf-8")), service, revision)
        except (SmokeError, ValueError, UnicodeDecodeError) as error:
            failures = [str(error)]
        if not failures:
            print(f"ok  {url} reports {service} at {revision or 'any revision'}")
            return
        if attempt + 1 < attempts:
            sleeper(interval)
    raise SmokeError(
        f"{url} did not reach the expected revision after {attempts} attempt(s):\n  "
        + "\n  ".join(failures)
    )


def run_probes(
    origin: str,
    probes: list[dict],
    timeout: float,
    fetcher: Callable[[str, float], tuple[int, bytes]],
) -> None:
    failures: list[str] = []
    for probe in probes:
        path = probe["path"]
        url = f"{origin}{path}"
        try:
            status, _ = fetcher(url, timeout)
        except SmokeError as error:
            failures.append(str(error))
            continue
        failure = evaluate(path, probe["expect"], status)
        if failure:
            failures.append(failure)
        else:
            print(f"ok  {url} answered {status} ({probe['expect']})")
    if failures:
        raise SmokeError("live surface checks failed:\n  " + "\n  ".join(failures))


def normalized_origin(origin: str) -> str:
    origin = origin.strip().rstrip("/")
    if not origin.startswith(("https://", "http://127.0.0.1", "http://localhost")):
        raise SmokeError(
            "origin must be HTTPS, or a loopback address for local verification"
        )
    return origin


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True, help="Live base URL, no trailing slash")
    parser.add_argument("--service", required=True, help="Expected health `service` value")
    parser.add_argument(
        "--revision",
        default=None,
        help="Full 40-character commit the origin must be serving",
    )
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--attempts", type=int, default=40)
    parser.add_argument("--interval", type=float, default=15.0)
    parser.add_argument("--timeout", type=float, default=15.0)
    args = parser.parse_args(argv)

    try:
        origin = normalized_origin(args.origin)
        revision = args.revision.strip().lower() if args.revision else None
        if revision in {"", "unknown"}:
            revision = None
        if revision is not None and not REVISION_RE.fullmatch(revision):
            raise SmokeError(
                "--revision must be a full 40-character commit SHA so the gate "
                "cannot be satisfied by an arbitrary string"
            )
        service = service_manifest(load_manifest(args.manifest), args.service)
        await_revision(
            origin,
            service["health_path"],
            args.service,
            revision,
            max(args.attempts, 1),
            args.interval,
            args.timeout,
            fetch,
            time.sleep,
        )
        run_probes(origin, service["probes"], args.timeout, fetch)
    except SmokeError as error:
        print(f"post-deploy smoke failed: {error}", file=sys.stderr)
        return 1
    print(f"post-deploy smoke passed for {args.service} at {origin}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

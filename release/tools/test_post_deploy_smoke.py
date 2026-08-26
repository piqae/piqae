from __future__ import annotations

import json
import unittest
from pathlib import Path

from post_deploy_smoke import (
    DEFAULT_MANIFEST,
    SmokeError,
    await_revision,
    check_health,
    evaluate,
    load_manifest,
    normalized_origin,
    run_probes,
    service_manifest,
)

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "openapi" / "piqae-v1.yaml"
WEB_ROUTES = ROOT / "apps" / "web" / "src" / "routes"
REVISION = "0123456789abcdef0123456789abcdef01234567"


def health(**overrides: object) -> bytes:
    payload = {
        "status": "ok",
        "service": "piqae-control-plane",
        "version": "0.1.17",
        "revision": REVISION,
    }
    payload.update(overrides)
    return json.dumps(payload).encode("utf-8")


class ProbeManifestTests(unittest.TestCase):
    """The manifest is curated, so it is held to the checked-in surface."""

    def test_every_api_probe_exists_in_the_published_contract(self) -> None:
        contract = CONTRACT.read_text(encoding="utf-8")
        manifest = load_manifest(DEFAULT_MANIFEST)
        probes = manifest["services"]["piqae-control-plane"]["probes"]
        self.assertTrue(probes)
        for probe in probes:
            with self.subTest(path=probe["path"]):
                self.assertIn(f"\n  {probe['path']}:\n", contract)

    def test_every_web_probe_has_a_route(self) -> None:
        manifest = load_manifest(DEFAULT_MANIFEST)
        for probe in manifest["services"]["piqae-web"]["probes"]:
            with self.subTest(path=probe["path"]):
                directory = WEB_ROUTES / probe["path"].strip("/")
                self.assertTrue(
                    any(directory.glob("+page.*")), f"{directory} has no page"
                )

    def test_the_workspace_endpoints_from_the_incident_are_probed(self) -> None:
        manifest = load_manifest(DEFAULT_MANIFEST)
        paths = {
            probe["path"]
            for probe in manifest["services"]["piqae-control-plane"]["probes"]
        }
        self.assertIn("/v1/workspaces/current", paths)
        self.assertIn("/v1/identity/me", paths)

    def test_destination_topology_is_probed_without_tenant_credentials(self) -> None:
        manifest = load_manifest(DEFAULT_MANIFEST)
        probes = {
            probe["path"]: probe["expect"]
            for probe in manifest["services"]["piqae-control-plane"]["probes"]
        }
        self.assertEqual(probes["/v1/physical-destinations"], "authenticated")
        self.assertEqual(probes["/v1/printer-routes"], "authenticated")

    def test_an_unknown_service_is_rejected(self) -> None:
        with self.assertRaises(SmokeError):
            service_manifest(load_manifest(DEFAULT_MANIFEST), "piqae-nonexistent")


class HealthTests(unittest.TestCase):
    def test_a_matching_document_passes(self) -> None:
        payload = json.loads(health())
        self.assertEqual(check_health(payload, "piqae-control-plane", REVISION), [])

    def test_a_different_revision_fails(self) -> None:
        payload = json.loads(health(revision="f" * 40))
        failures = check_health(payload, "piqae-control-plane", REVISION)
        self.assertEqual(len(failures), 1)
        self.assertIn("not serving the reviewed commit", failures[0])

    def test_a_different_service_fails(self) -> None:
        payload = json.loads(health(service="piqae-web"))
        self.assertTrue(check_health(payload, "piqae-control-plane", REVISION))

    def test_an_unknown_revision_is_not_accepted_as_a_match(self) -> None:
        payload = json.loads(health(revision="unknown"))
        self.assertTrue(check_health(payload, "piqae-control-plane", REVISION))


class ProbeEvaluationTests(unittest.TestCase):
    def test_the_incident_signature_fails(self) -> None:
        failure = evaluate("/v1/workspaces/current", "authenticated", 503)
        self.assertIsNotNone(failure)
        assert failure is not None
        self.assertIn("unavailable in this deployment", failure)

    def test_an_unauthenticated_tenant_endpoint_must_reject(self) -> None:
        self.assertIsNone(evaluate("/v1/jobs", "authenticated", 401))
        self.assertIsNone(evaluate("/v1/jobs", "authenticated", 403))
        bypass = evaluate("/v1/jobs", "authenticated", 200)
        assert bypass is not None
        self.assertIn("authentication bypass", bypass)

    def test_a_public_endpoint_must_answer(self) -> None:
        self.assertIsNone(evaluate("/v1/meta", "public", 200))
        self.assertIsNotNone(evaluate("/v1/meta", "public", 404))
        self.assertIsNotNone(evaluate("/v1/meta", "public", 500))


class OriginTests(unittest.TestCase):
    def test_plaintext_origins_are_refused(self) -> None:
        with self.assertRaises(SmokeError):
            normalized_origin("http://api.example.com")

    def test_trailing_slashes_are_removed(self) -> None:
        self.assertEqual(
            normalized_origin("https://api.example.com/"), "https://api.example.com"
        )


class PollingTests(unittest.TestCase):
    def test_it_waits_for_the_reviewed_revision(self) -> None:
        responses = [
            (200, health(revision="a" * 40)),
            (200, health(revision="a" * 40)),
            (200, health()),
        ]
        slept: list[float] = []

        def fetcher(_url: str, _timeout: float) -> tuple[int, bytes]:
            return responses.pop(0)

        await_revision(
            "https://api.example.com",
            "/v1/health",
            "piqae-control-plane",
            REVISION,
            attempts=5,
            interval=0.0,
            timeout=1.0,
            fetcher=fetcher,
            sleeper=slept.append,
        )
        self.assertEqual(len(slept), 2)

    def test_it_gives_up_on_a_revision_that_never_arrives(self) -> None:
        def fetcher(_url: str, _timeout: float) -> tuple[int, bytes]:
            return 200, health(revision="a" * 40)

        with self.assertRaises(SmokeError) as raised:
            await_revision(
                "https://api.example.com",
                "/v1/health",
                "piqae-control-plane",
                REVISION,
                attempts=2,
                interval=0.0,
                timeout=1.0,
                fetcher=fetcher,
                sleeper=lambda _seconds: None,
            )
        self.assertIn("did not reach the expected revision", str(raised.exception))

    def test_probe_failures_are_reported_together(self) -> None:
        def fetcher(url: str, _timeout: float) -> tuple[int, bytes]:
            return (503, b"") if url.endswith("/v1/workspaces/current") else (401, b"")

        with self.assertRaises(SmokeError) as raised:
            run_probes(
                "https://api.example.com",
                [
                    {"path": "/v1/workspaces/current", "expect": "authenticated"},
                    {"path": "/v1/jobs", "expect": "authenticated"},
                ],
                timeout=1.0,
                fetcher=fetcher,
            )
        self.assertIn("/v1/workspaces/current", str(raised.exception))
        self.assertNotIn("/v1/jobs answered", str(raised.exception))


if __name__ == "__main__":
    unittest.main()

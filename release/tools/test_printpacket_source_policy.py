#!/usr/bin/env python3
"""Tests for the canonical-only PrintPacket source policy."""

from __future__ import annotations

import unittest

import check_printpacket_source_policy as policy


class PrintPacketSourcePolicyTest(unittest.TestCase):
    def test_canonical_source_passes(self) -> None:
        self.assertEqual(
            policy.violations_for(
                "src/packet.ts", 'const format = "printpacket/v1";'
            ),
            [],
        )

    def test_removed_wire_identifier_is_limited_to_negative_fixtures(self) -> None:
        source = f'const removed = "{policy.REMOVED_WIRE_IDENTIFIER}";'
        self.assertTrue(policy.violations_for("src/packet.ts", source))
        self.assertEqual(
            policy.violations_for("apps/mcp/tests/server.test.ts", source), []
        )

    def test_predecessor_symbols_routes_and_tables_fail(self) -> None:
        fragments = [
            "Business" + "DocumentV1",
            "client." + "business" + "Documents",
            "CREATE TABLE " + "business" + "_document_resources",
            'route("/v1/' + "business" + '-document-renders")',
        ]
        for fragment in fragments:
            with self.subTest(fragment=fragment):
                self.assertTrue(policy.violations_for("src/packet.ts", fragment))

    def test_predecessor_migration_filename_fails(self) -> None:
        path = "migrations/postgres/0038_" + "business" + "_document_cutover.sql"
        self.assertTrue(policy.violations_for(path, "SELECT 1;"))


if __name__ == "__main__":
    unittest.main()

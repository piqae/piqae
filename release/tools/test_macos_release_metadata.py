from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from macos_release_metadata import artifact, validate_appcast


class MacosReleaseMetadataTests(unittest.TestCase):
    def test_metadata_preserves_shell_sensitive_prose_without_shell_quoting(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory) / "installer.pkg"
            installer.write_bytes(b"verified installer")
            result = artifact("1.2.3", "42", installer, "2026-08-05T00:00:00Z")
        self.assertIn("Sparkle's boundary", result["artifact"]["notes"][-1])
        self.assertEqual(64, len(result["artifact"]["sha256"]))

    def test_appcast_requires_exact_version_build_url_and_signature(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            appcast = Path(directory) / "appcast.xml"
            appcast.write_text(
                """<?xml version="1.0"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel><item>
    <sparkle:shortVersionString>1.2.3</sparkle:shortVersionString>
    <sparkle:version>42</sparkle:version>
    <enclosure url="https://downloads.piqae.com/releases/stable/piqae-macos-1.2.3-42-update.zip" sparkle:edSignature="signed" />
  </item></channel>
</rss>"""
            )
            self.assertTrue(validate_appcast(appcast, "1.2.3", "42").endswith("update.zip"))
            with self.assertRaisesRegex(ValueError, "does not match"):
                validate_appcast(appcast, "1.2.4", "42")


if __name__ == "__main__":
    unittest.main()

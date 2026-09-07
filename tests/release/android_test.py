#!/usr/bin/env python3
"""Android installer version ordering and stale-candidate regression checks."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("android_release", ROOT / "tools/android/release.py")
android = importlib.util.module_from_spec(spec)
spec.loader.exec_module(android)


class AndroidReleaseTest(unittest.TestCase):
    def test_installer_order_and_existing_acceptance_upgrade(self):
        self.assertGreater(android.version_code("0.1.26"), 1016)
        versions = ["0.1.26-alpha.1", "0.1.26-alpha.29", "0.1.26-beta.1",
                    "0.1.26-beta.29", "0.1.26-rc.1", "0.1.26-rc.38", "0.1.26",
                    "0.1.27-alpha.1", "0.1.999", "0.2.0-alpha.1", "1.0.0-alpha.1"]
        codes = [android.version_code(version) for version in versions]
        self.assertEqual(codes, sorted(set(codes)))
        for invalid in ["0.1.26+build", "0.1.26-rc.39", "0.1.26-alpha.30",
                        "0.1.26-rc.0", "0.100.0", "0.1.1000", "210.0.0", "01.1.26"]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                android.version_code(invalid)

    def test_stale_embedded_identity_is_rejected_before_signing_tools(self):
        with tempfile.TemporaryDirectory() as temporary:
            apk = Path(temporary) / "stale.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("assets/zterm-build.json", "{}")
            with self.assertRaisesRegex(ValueError, "Packaged build identity"):
                android.inspect(apk, Path("/missing-sdk"), "a" * 40, False)


if __name__ == "__main__":
    unittest.main()

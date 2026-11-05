import runpy
from pathlib import Path
import subprocess
import sys
import unittest


package = runpy.run_path(str(Path(__file__).resolve().parents[1] / "scripts/package-release"))
ROOT = Path(__file__).resolve().parents[1]
notes = runpy.run_path(str(ROOT / "scripts/release-notes"))
BODY = b"### Upgrade effort\n\nMicro: no user changes needed.\n\nUpdated the README with a clearer introduction, quick-start examples, and links to detailed guides.\n\nNo functional changes.\n"


class ReleasePackageTests(unittest.TestCase):
    def test_release_notes_cli_emits_exact_body_without_history_or_heading(self):
        result = subprocess.run([sys.executable, str(ROOT / "scripts/release-notes"), "1.0.1"], check=True, capture_output=True, timeout=5)
        self.assertEqual(result.stdout, BODY)
        self.assertEqual(result.stderr, b"")

    def test_release_notes_select_only_requested_section(self):
        changelog = "# Changelog\n\n## 1.0.2\n\nNewer.\n\n## 1.0.1\n\nFix.\n\n### Details\n\nText.\n\n## 1.0.0\n\nOlder.\n"
        self.assertEqual(notes["extract_notes"](changelog, "1.0.1"), "Fix.\n\n### Details\n\nText.\n")
        self.assertEqual(notes["extract_notes"](changelog, "1.0.0"), "Older.\n")

    def test_release_notes_reject_missing_empty_or_duplicate_version(self):
        for changelog in ("## 1.0.0\nOld.\n", "## 1.0.1\n \t\n## 1.0.0\nOld.\n", "## 1.0.1\n", "## 1.0.1\nFix.\n## 1.0.1\nOther.\n"):
            with self.subTest(changelog=changelog), self.assertRaises(ValueError):
                notes["extract_notes"](changelog, "1.0.1")
        with self.assertRaises(ValueError):
            notes["extract_notes"]("## 01.0.1\nFix.\n", "01.0.1")

    def test_tag_must_be_canonical_and_match_package(self):
        package["validate_tag"]("v1.0.0", "1.0.0")
        for tag in ("v1.1.0", "1.0.0", "v01.0.0", "v1.0.0-rc1", "v1.0.0\n", "v1.0.0/../x", "$(id)"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                package["validate_tag"](tag, "1.0.0")

    def test_only_system_dynamic_libraries_are_distributable(self):
        package["validate_linkage"]("reswho:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n\t/System/Library/Frameworks/Security.framework/Versions/A/Security (compatibility version 1.0.0)")
        for path in ("/opt/homebrew/opt/curl/lib/libcurl.dylib", "/usr/local/lib/libcurl.dylib", "@rpath/libcurl.dylib", "/tmp/test.dylib"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                package["validate_linkage"]("reswho:\n\t" + path + " (compatibility version 1.0.0)")
        with self.assertRaises(ValueError):
            package["validate_linkage"]("reswho:\n")


if __name__ == "__main__":
    unittest.main()

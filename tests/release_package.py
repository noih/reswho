import runpy
from pathlib import Path
import unittest


package = runpy.run_path(str(Path(__file__).resolve().parents[1] / "scripts/package-release"))


class ReleasePackageTests(unittest.TestCase):
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

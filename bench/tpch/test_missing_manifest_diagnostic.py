"""Missing TPC-H files have actionable diagnostics, not unsafe-file messages."""
import tempfile
import unittest
from pathlib import Path

from data_manifest import inspect_table


class MissingManifestDiagnosticTests(unittest.TestCase):
    def test_missing_file_has_distinct_error(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "does not exist"):
                inspect_table(Path(root) / "region.tsv", "region")

    def test_broken_symlink_is_still_unsafe(self):
        with tempfile.TemporaryDirectory() as root:
            link = Path(root) / "region.tsv"
            link.symlink_to(Path(root) / "nowhere.tsv")
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                inspect_table(link, "region")


if __name__ == "__main__":
    unittest.main()

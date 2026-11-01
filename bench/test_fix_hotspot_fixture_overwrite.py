import tempfile
import unittest
from pathlib import Path

from bench.joins.hotspot import write_fixture


class HotspotFixtureOverwriteTests(unittest.TestCase):
    def test_existing_dimension_file_stays_untouched_and_no_facts_are_written(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            (folder / "dimensions.tsv").write_bytes(b"keep")
            with self.assertRaisesRegex(FileExistsError, "overwrite"):
                write_fixture(folder, 3, 2, 50)
            self.assertEqual((folder / "dimensions.tsv").read_bytes(), b"keep")
            self.assertFalse((folder / "facts.tsv").exists())

    def test_new_files_still_generate_expected_rows(self):
        with tempfile.TemporaryDirectory() as root:
            counts = write_fixture(root, 3, 2, 50)
            self.assertEqual(sum(counts.values()), 3)
            self.assertEqual(len((Path(root) / "facts.tsv").read_text().splitlines()), 4)


if __name__ == "__main__":
    unittest.main()

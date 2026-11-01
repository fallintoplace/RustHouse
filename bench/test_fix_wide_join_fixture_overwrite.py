import tempfile
import unittest
from pathlib import Path

from bench.joins.wide_build import write_dimensions


class WideJoinFixtureOverwriteTests(unittest.TestCase):
    def test_existing_wide_fixture_is_preserved_before_narrow_is_created(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            (folder / "wide.tsv").write_bytes(b"keep this")
            with self.assertRaisesRegex(FileExistsError, "overwrite"):
                write_dimensions(folder, 2, 5)
            self.assertEqual((folder / "wide.tsv").read_bytes(), b"keep this")
            self.assertFalse((folder / "narrow.tsv").exists())

    def test_new_fixtures_still_have_same_join_keys(self):
        with tempfile.TemporaryDirectory() as root:
            narrow, wide = write_dimensions(root, 3, 5)
            self.assertEqual(narrow.read_text().splitlines(), ["id", "0", "1", "2"])
            self.assertEqual(wide.read_text().splitlines(),
                             ["id\tpayload", "0\txxxxx", "1\txxxxx", "2\txxxxx"])


if __name__ == "__main__":
    unittest.main()

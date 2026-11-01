import tempfile
import unittest
from pathlib import Path

from bench.analytics.fixture import generate


class AnalyticsFixtureOverwriteTests(unittest.TestCase):
    def test_existing_fixture_does_not_get_overwritten_or_partially_created(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / "hits.sqlite").write_bytes(b"keep my data")
            with self.assertRaisesRegex(FileExistsError, "overwrite"):
                generate(folder, rows=2)
            self.assertEqual((folder / "hits.sqlite").read_bytes(), b"keep my data")
            self.assertFalse((folder / "hits.tsv").exists())
            self.assertFalse((folder / "create.sql").exists())

    def test_new_fixture_still_generates(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(generate(tmp, rows=2)["rows"], 2)
            self.assertTrue((Path(tmp) / "hits.sqlite").is_file())
            self.assertTrue((Path(tmp) / "hits.tsv").is_file())


if __name__ == "__main__":
    unittest.main()

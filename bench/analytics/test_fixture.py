import sqlite3
from contextlib import closing
import tempfile
import unittest
from pathlib import Path
from fixture import FIELDS, generate, records


class ClickBenchFixtureTests(unittest.TestCase):
    def test_schema_and_sqlite_reference_agree_on_rows(self):
        with tempfile.TemporaryDirectory() as root:
            result = generate(root, rows=300, seed=11)
            self.assertFalse(result["official_clickbench"])
            path = Path(root)
            self.assertEqual(len((path / "hits.tsv").read_text().splitlines()), 301)
            self.assertEqual(len(FIELDS), 7)
            with closing(sqlite3.connect(path / "hits.sqlite")) as db:
                self.assertEqual(db.execute("SELECT COUNT(*) FROM hits").fetchone()[0], 300)
                self.assertGreater(db.execute("SELECT COUNT(DISTINCT UserID) FROM hits").fetchone()[0], 0)
            self.assertIn("COPY hits", (path / "copy.sql").read_text())

    def test_seed_is_repeatable_and_empty_fixture_rejected(self):
        self.assertEqual(list(records(10, 7)), list(records(10, 7)))
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "rows"):
                generate(root, rows=0)


class SyntheticFixtureParameterTests(unittest.TestCase):
    def test_invalid_row_and_seed_controls_fail_without_creating_files(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "fixture"
            for options in ({"rows": True}, {"rows": 10.5},
                            {"seed": True}, {"seed": 1.25},
                            {"seed": -1}, {"seed": 1 << 64}):
                with self.subTest(options=options):
                    with self.assertRaisesRegex(ValueError, "rows|seed"):
                        generate(folder, **options)
                    self.assertFalse(folder.exists())

if __name__ == "__main__":
    unittest.main()

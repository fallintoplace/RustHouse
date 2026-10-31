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

    def test_fixture_bytes_and_sqlite_rows_cross_multiple_batches(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, rows=1025, seed=17)
            path = Path(root)
            actual = (path / "hits.tsv").read_text(encoding="utf-8")
            expected = "\t".join(name for name, _, _ in FIELDS) + "\n"
            expected += "".join("\t".join(map(str, record)) + "\n"
                                for record in records(1025, 17))
            self.assertEqual(actual, expected)
            with closing(sqlite3.connect(path / "hits.sqlite")) as connection:
                self.assertEqual(
                    connection.execute("SELECT count(*) FROM hits").fetchone()[0], 1025
                )

    def test_seed_is_repeatable_and_empty_fixture_rejected(self):
        self.assertEqual(list(records(10, 7)), list(records(10, 7)))
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "rows"):
                generate(root, rows=0)


class SyntheticFixtureParameterTests(unittest.TestCase):
    def test_rejects_symlinked_outputs_before_writing(self):
        for name in ("hits.tsv", "hits.sqlite", "create.sql", "copy.sql"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as root:
                directory = Path(root) / "fixture"
                directory.mkdir()
                protected = Path(root) / "protected.txt"
                protected.write_text("keep me")
                (directory / name).symlink_to(protected)
                with self.assertRaisesRegex(ValueError, "symlinked fixture output"):
                    generate(directory, rows=5)
                self.assertEqual(protected.read_text(), "keep me")
                self.assertFalse((directory / ("hits.sqlite" if name != "hits.sqlite" else "hits.tsv")).exists())

    def test_rejects_symlinked_fixture_root(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root) / "real"
            directory.mkdir()
            link = Path(root) / "alias"
            link.symlink_to(directory, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "fixture root"):
                generate(link, rows=5)
            self.assertEqual(list(directory.iterdir()), [])

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

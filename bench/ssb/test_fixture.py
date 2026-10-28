import sqlite3
from contextlib import closing
import tempfile
import unittest
from pathlib import Path
from fixture import DATE_KEYS, MAX_FACTS, TABLES, generate


class StarFixtureTests(unittest.TestCase):
    def test_dates_and_facts_have_valid_references(self):
        with tempfile.TemporaryDirectory() as root:
            report = generate(root, 123, 7)
            path = Path(root)
            with closing(sqlite3.connect(path / "reference.sqlite")) as db:
                self.assertEqual(db.execute("SELECT COUNT(*) FROM lineorder").fetchone()[0], 123)
                self.assertEqual(db.execute("SELECT COUNT(*) FROM date_dim").fetchone()[0],
                                 len(DATE_KEYS))
                unmatched = db.execute(
                    "SELECT count(*) FROM lineorder l LEFT JOIN date_dim d "
                    "ON l.lo_orderdate = d.d_datekey WHERE d.d_datekey IS NULL"
                ).fetchone()[0]
                self.assertEqual(unmatched, 0)
            self.assertEqual(len((path / "lineorder.tsv").read_text().splitlines()), 124)
            self.assertFalse(report["official_ssb"])
            self.assertIn("CREATE TABLE", (path / "date_dim.sql").read_text())

    def test_regenerating_fixture_replaces_old_reference_rows(self):
        with tempfile.TemporaryDirectory() as root:
            generate(root, 20, 2)
            generate(root, 4, 3)
            with closing(sqlite3.connect(Path(root) / "reference.sqlite")) as db:
                self.assertEqual(
                    db.execute("SELECT COUNT(*) FROM lineorder").fetchone()[0], 4
                )
                self.assertEqual(
                    db.execute("SELECT COUNT(*) FROM date_dim").fetchone()[0],
                    len(DATE_KEYS)
                )
            self.assertEqual(
                len((Path(root) / "lineorder.tsv").read_text().splitlines()), 5
            )

    def test_streamed_fact_reference_spans_more_than_buffered_join_limit(self):
        with tempfile.TemporaryDirectory() as root:
            report = generate(root, rows=32769, seed=9)
            with closing(sqlite3.connect(Path(root) / "reference.sqlite")) as db:
                self.assertEqual(
                    db.execute("SELECT count(*), count(DISTINCT lo_orderkey) FROM lineorder").fetchone(),
                    (32769, 32769)
                )
            self.assertEqual(report["rows"], 32769)
            with (Path(root) / "lineorder.tsv").open() as stream:
                self.assertEqual(sum(1 for _ in stream), 32770)

    def test_seed_is_deterministic_and_input_bounded(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            generate(a, 70, 14)
            generate(b, 70, 14)
            self.assertEqual((Path(a) / "lineorder.tsv").read_bytes(),
                             (Path(b) / "lineorder.tsv").read_bytes())
            with self.assertRaisesRegex(ValueError, "rows"):
                generate(a, MAX_FACTS + 1)


if __name__ == "__main__":
    unittest.main()

import sqlite3
import tempfile
import unittest
from pathlib import Path
from fixture import DATE_KEYS, MAX_FACTS, TABLES, generate


class StarFixtureTests(unittest.TestCase):
    def test_dates_and_facts_have_valid_references(self):
        with tempfile.TemporaryDirectory() as root:
            report = generate(root, 123, 7)
            path = Path(root)
            with sqlite3.connect(path / "reference.sqlite") as db:
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

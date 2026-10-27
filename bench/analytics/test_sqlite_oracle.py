import tempfile
import unittest
from pathlib import Path
import sqlite3
from sqlite_oracle import assert_matches, cell_equal, parse_rows, sqlite_query


class SqliteOracleTests(unittest.TestCase):
    def test_large_integers_must_match_exactly(self):
        value = 9007199254740993
        self.assertTrue(cell_equal(str(value), value))
        self.assertFalse(cell_equal(str(value + 1), value))

    def test_float_approximation_and_mismatch(self):
        self.assertTrue(cell_equal("100.00000000001", 100.0))
        self.assertFalse(cell_equal("100.2", 100.0))
        self.assertFalse(cell_equal("nan", 100.0))
        with self.assertRaisesRegex(ValueError, "differs"):
            assert_matches(b"1\tno\n", [(1, "yes")])

    def test_ties_compare_as_a_multiset(self):
        with self.assertRaisesRegex(ValueError, "row 1"):
            assert_matches(b"B\t2\nA\t1\n", [("A", 1), ("B", 2)])
        assert_matches(b"B\t2\nA\t1\n", [("A", 1), ("B", 2)], unordered=True)
        self.assertEqual(parse_rows(b""), [])

    def test_sqlite_query(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "db.sqlite"
            with sqlite3.connect(path) as db:
                db.execute("CREATE TABLE t (x INTEGER)")
                db.execute("INSERT INTO t VALUES (3)")
            self.assertEqual(sqlite_query(path, "SELECT x FROM t"), [(3,)])


if __name__ == "__main__":
    unittest.main()

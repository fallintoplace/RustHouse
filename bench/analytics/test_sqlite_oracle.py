import tempfile
import unittest
from pathlib import Path
import sqlite3
from contextlib import closing
from sqlite_oracle import assert_matches, cell_equal, decode_tsv_value, parse_rows, sqlite_query


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

    def test_literal_null_text_is_not_a_null_cell(self):
        self.assertFalse(cell_equal("NULL", None))
        self.assertTrue(cell_equal(r"\N", None))
        self.assertTrue(cell_equal("NULL", "NULL"))
        with self.assertRaisesRegex(ValueError, "row 1"):
            assert_matches(b"NULL\n", [(None,)])

    def test_tsv_escaped_text_matches_sqlite_typed_string_values(self):
        self.assertTrue(cell_equal(r"left\tright", "left\tright"))
        self.assertTrue(cell_equal(r"C:\\data", r"C:\data"))
        self.assertTrue(cell_equal(r"\xC3\xA9", "é"))
        self.assertTrue(cell_equal(r"\\N", r"\N"))
        self.assertFalse(cell_equal(r"\N", r"\N"))
        self.assertFalse(cell_equal(r"\N", "NULL"))
        self.assertEqual(decode_tsv_value(r"\N"), None)
        self.assertEqual(decode_tsv_value(r"\\N"), r"\N")
        self.assertEqual(decode_tsv_value(r"line\nnext"), "line\nnext")
        self.assertEqual(decode_tsv_value(r"\xC3\xA9"), "é")
        assert_matches(
            b"left\\tright\tC:\\\\data\t\\xC3\\xA9\n",
            [("left\tright", r"C:\data", "é")],
        )

    def test_rejects_bad_tsv_escapes_instead_of_accepting_untyped_text(self):
        for value in [r"\q", r"\x1", r"\xGG", "ends\\", r"\xFF"]:
            with self.subTest(value=value):
                self.assertFalse(cell_equal(value, value))
                with self.assertRaises((ValueError, UnicodeError)):
                    decode_tsv_value(value)
        self.assertFalse(cell_equal(r"\0", "0"))
        self.assertTrue(cell_equal(r"\0", "\x00"))

    def test_ties_compare_as_a_multiset(self):
        with self.assertRaisesRegex(ValueError, "row 1"):
            assert_matches(b"B\t2\nA\t1\n", [("A", 1), ("B", 2)])
        assert_matches(b"B\t2\nA\t1\n", [("A", 1), ("B", 2)], unordered=True)
        self.assertEqual(parse_rows(b""), [])

    def test_unicode_line_separators_stay_inside_tsv_fields(self):
        text = "first\u2028second\t7\n"
        self.assertEqual(parse_rows(text.encode("utf-8")),
                         [["first\u2028second", "7"]])
        self.assertEqual(parse_rows(b"a\t1\r\nb\t2\r\n"),
                         [["a", "1"], ["b", "2"]])

    def test_sqlite_reference_cannot_be_created_by_query(self):
        with tempfile.TemporaryDirectory() as root:
            missing = Path(root) / "missing-reference.sqlite"
            with self.assertRaises(sqlite3.OperationalError):
                sqlite_query(missing, "SELECT 1")
            self.assertFalse(missing.exists())

    def test_sqlite_query(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "db.sqlite"
            with closing(sqlite3.connect(path)) as db, db:
                db.execute("CREATE TABLE t (x INTEGER)")
                db.execute("INSERT INTO t VALUES (3)")
            self.assertEqual(sqlite_query(path, "SELECT x FROM t"), [(3,)])


    def test_sqlite_oracle_limits_materialized_reference_rows(self):
        from unittest import mock
        with mock.patch("sqlite_oracle.sqlite3.connect") as connect:
            cursor = connect.return_value.execute.return_value
            cursor.fetchmany.return_value = [(1,)] * 100_001
            with self.assertRaisesRegex(ValueError, "100000 rows"):
                sqlite_query("sample.sqlite", "SELECT id FROM hits")
            cursor.fetchmany.assert_called_once_with(100_001)
            cursor.fetchall.assert_not_called()


    def test_large_unordered_integer_rows_use_exact_multiplicity(self):
        rows = [(index % 23, index % 5) for index in range(3000)]
        actual = "".join(f"{a}\t{b}\n" for a, b in reversed(rows)).encode()
        assert_matches(actual, rows, unordered=True)
        bad = actual.replace(b"0\t0\n", b"999\t0\n", 1)
        with self.assertRaisesRegex(ValueError, "multiset"):
            assert_matches(bad, rows, unordered=True)
        with self.assertRaisesRegex(ValueError, "multiset"):
            assert_matches(b"01\n", [(1,)], unordered=True)

if __name__ == "__main__":
    unittest.main()

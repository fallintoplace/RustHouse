import sqlite3
from contextlib import closing
import tempfile
import unittest
from pathlib import Path

from schema import names
from sqlite_load import convert_cell, load_database


class SQLiteLoadTests(unittest.TestCase):
    def test_fixed_point_data_load_is_verified(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "region.tsv").write_text(
                "\t".join(names("region")) + "\n"
                + "1\tEUROPE\tC:\\\\data\n"
                + "2\tASIA\tcomment\n",
                encoding="utf-8")
            output = directory / "ref.sqlite"
            result = load_database(directory, output, ["region"], batch_size=1)
            self.assertFalse(result["official_tpch_score"])
            self.assertEqual(result["tables"][0]["rows"], 2)
            with closing(sqlite3.connect(output)) as connection:
                self.assertEqual(connection.execute(
                    "SELECT COUNT(*) FROM region").fetchone()[0], 2)
                self.assertEqual(connection.execute(
                    "SELECT r_comment FROM region WHERE r_regionkey=1").fetchone()[0],
                                 "C:\\data")

    def test_rejects_bad_input_atomically(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "region.tsv").write_text("incorrect\n")
            database = directory / "ref.sqlite"
            with self.assertRaisesRegex(ValueError, "header"):
                load_database(directory, database, ["region"])
            self.assertFalse(database.exists())

    def test_rejects_sqlite_integer_overflow(self):
        with self.assertRaisesRegex(ValueError, "64-bit"):
            convert_cell(str(2**63), "UInt64")
        with self.assertRaisesRegex(ValueError, "negative"):
            convert_cell("-1", "UInt64")


if __name__ == "__main__":
    unittest.main()

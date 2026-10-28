import os
import sqlite3
from contextlib import closing
import tempfile
import unittest
from pathlib import Path
from unittest import mock

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

    def test_does_not_overwrite_competing_reference_file(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "region.tsv").write_text(
                "\t".join(names("region")) + "\n1\tEUROPE\tcomment\n"
            )
            target = directory / "reference.db"
            link = os.link

            def competing_link(source, destination):
                target.write_bytes(b"other process")
                return link(source, destination)

            with mock.patch("sqlite_load.os.link", side_effect=competing_link):
                with self.assertRaises(FileExistsError):
                    load_database(directory, target, ["region"])
            self.assertEqual(target.read_bytes(), b"other process")

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

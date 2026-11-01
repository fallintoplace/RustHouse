import sqlite3
from contextlib import closing
import tempfile
import unittest
from pathlib import Path

from sqlite_goldens import generate, tsv_cell


class SQLiteGoldenTests(unittest.TestCase):
    def test_writes_selected_queries_as_headerless_tsv(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            database = base / "data.sqlite"
            with closing(sqlite3.connect(database)) as connection, connection:
                connection.execute("CREATE TABLE demo (id INTEGER, title TEXT)")
                connection.execute("INSERT INTO demo VALUES (1, ?)", ("A\\B",))
                connection.execute("INSERT INTO demo VALUES (2, NULL)")
            query_dir = base / "queries"
            query_dir.mkdir()
            (query_dir / "q01.sql").write_text("SELECT id, title FROM demo ORDER BY id;")
            (query_dir / "q06.sql").write_text("SELECT COUNT(*) FROM demo;")
            result = generate(database, query_dir, base / "goldens", only="1,6")
            self.assertEqual([x["rows"] for x in result["results"]], [2, 1])
            self.assertEqual((base / "goldens" / "q01.tsv").read_bytes(),
                             b"1\tA\\\\B\n2\tNULL\n")
            self.assertEqual((base / "goldens" / "q06.tsv").read_bytes(), b"2\n")
            with self.assertRaisesRegex(ValueError, "overwrite"):
                generate(database, query_dir, base / "goldens")

    def test_rejects_nonfinite_float_references(self):
        for value in (float("nan"), float("inf"), -float("inf")):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "nonfinite"):
                tsv_cell(value)
        self.assertEqual(tsv_cell(1.25), "1.25")


    def test_null_and_unsupported_query(self):
        self.assertEqual(tsv_cell(None, r"\N"), r"\N")
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            database = base / "db.sqlite"
            with closing(sqlite3.connect(database)) as connection, connection:
                pass
            query_dir = base / "queries"
            query_dir.mkdir()
            (query_dir / "q01.sql").write_text("SELECT * FROM non_existing_table;")
            with self.assertRaisesRegex(ValueError, "q01"):
                generate(database, query_dir, base / "goldens")


class SQLiteGoldenOutputBudgetTests(unittest.TestCase):
    def test_existing_tmp_symlink_is_never_followed(self):
        from sqlite_goldens import write_one
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            destination = base / "q01.tsv"
            protected = base / "protected.txt"
            protected.write_text("keep me")
            (base / "q01.tsv.tmp").symlink_to(protected)
            with closing(sqlite3.connect(":memory:")) as connection:
                self.assertEqual(write_one(connection, "SELECT 1", destination), 1)
            self.assertEqual(protected.read_text(), "keep me")
            self.assertEqual(destination.read_bytes(), b"1\n")


    def test_does_not_overwrite_reference_created_after_preflight(self):
        from sqlite_goldens import write_one
        from unittest.mock import patch
        import os
        with tempfile.TemporaryDirectory() as root:
            destination = Path(root) / "q01.tsv"
            original_link = os.link

            def concurrent_create(source, target):
                destination.write_bytes(b"winner\n")
                return original_link(source, target)

            with closing(sqlite3.connect(":memory:")) as connection:
                with patch("sqlite_goldens.os.link", side_effect=concurrent_create):
                    with self.assertRaises(FileExistsError):
                        write_one(connection, "SELECT 1", destination)
            self.assertEqual(destination.read_bytes(), b"winner\n")
            self.assertEqual(list(Path(root).glob("*.tmp")), [])

    def test_explicit_overwrite_replaces_existing_reference(self):
        from sqlite_goldens import write_one
        with tempfile.TemporaryDirectory() as root:
            destination = Path(root) / "q01.tsv"
            destination.write_bytes(b"old\n")
            with closing(sqlite3.connect(":memory:")) as connection:
                write_one(connection, "SELECT 2", destination, overwrite=True)
            self.assertEqual(destination.read_bytes(), b"2\n")

    def test_enforces_bytes_and_does_not_publish_partial_reference(self):
        from sqlite_goldens import write_one
        with tempfile.TemporaryDirectory() as root:
            destination = Path(root) / "q01.tsv"
            with closing(sqlite3.connect(":memory:")) as connection:
                with self.assertRaisesRegex(ValueError, "byte limit"):
                    write_one(connection, "SELECT 'ééééééééé'", destination,
                              max_output_bytes=4)
            self.assertFalse(destination.exists())

if __name__ == "__main__":
    unittest.main()

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

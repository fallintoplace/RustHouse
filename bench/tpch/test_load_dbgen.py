import tempfile
import unittest
from pathlib import Path

from load_dbgen import inspect_tsv, load_tables
from schema import TABLES, names


class DbgenLoadTests(unittest.TestCase):
    def test_import_selected_tables_and_verify_exact_counts(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)
            for table in ("region", "nation"):
                (source / f"{table}.tsv").write_text(
                    "\t".join(names(table)) + "\n" + "\t".join(["1"] * len(TABLES[table])) + "\n"
                )
            statements = []
            def execute(sql):
                statements.append(sql)
                return b"1\n" if sql.startswith("SELECT count") else b""
            report = load_tables(Path("fake"), source / "db", source,
                                 ["region", "nation"], execute=execute)
            self.assertEqual([x["rows"] for x in report["tables"]], [1, 1])
            self.assertEqual(len(statements), 6)
            self.assertIn("COPY region", statements[1])
            self.assertEqual(report["timing_scope"], "per-table COPY statement only")

    def test_preflight_fails_before_writing_wrong_header(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            path.write_text("wrong\n1\n")
            with self.assertRaisesRegex(ValueError, "header"):
                inspect_tsv(path, "region")
            called = []
            with self.assertRaisesRegex(ValueError, "header"):
                load_tables(Path("fake"), Path(root) / "db", Path(root), ["region"],
                            execute=lambda sql: called.append(sql))
            self.assertEqual(called, [])

    def test_disallows_duplicate_table_selection(self):
        with self.assertRaisesRegex(ValueError, "unique"):
            load_tables(Path("fake"), Path("db"), Path("input"), ["part", "part"])


if __name__ == "__main__":
    unittest.main()

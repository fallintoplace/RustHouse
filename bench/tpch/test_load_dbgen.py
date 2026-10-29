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

    def test_preflight_rejects_malformed_records(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            header = "\t".join(names("region")).encode("utf-8") + b"\n"
            for row in (b"1\tEUROPE\n", b"\n", b"1\tEUROPE\tcomment", b"1\tEUROPE\tcomment\textra\n"):
                with self.subTest(row=row):
                    path.write_bytes(header + row)
                    with self.assertRaisesRegex(ValueError, "malformed"):
                        inspect_tsv(path, "region")

    def test_preflight_counts_binary_records_with_crlf(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            header = "\t".join(names("region")).encode("utf-8")
            path.write_bytes(header + b"\r\n" + b"1\tEUROPE\tname\r\n" + b"2\tASIA\tname\n")
            self.assertEqual(inspect_tsv(path, "region"), 2)

    def test_disallows_empty_table_selection(self):
        with self.assertRaisesRegex(ValueError, "nonempty"):
            load_tables(Path("fake"), Path("db"), Path("input"), [])

    def test_disallows_duplicate_table_selection(self):
        with self.assertRaisesRegex(ValueError, "unique"):
            load_tables(Path("fake"), Path("db"), Path("input"), ["part", "part"])


class BoundedDbgenLoaderTests(unittest.TestCase):
    def test_rejects_oversized_data_row_before_copying(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "region.tsv"
            path.write_bytes(("\t".join(names("region")) + "\n").encode() +
                             b"1\tEUROPE\t" + b"x" * 100 + b"\n")
            with patch("load_dbgen.MAX_RECORD_BYTES", 64):
                with self.assertRaisesRegex(ValueError, "malformed"):
                    inspect_tsv(path, "region")

if __name__ == "__main__":
    unittest.main()

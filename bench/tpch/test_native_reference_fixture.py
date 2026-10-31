import tempfile
import unittest
from pathlib import Path
from subprocess import CompletedProcess

from native_reference_fixture import (
    convert_native_table, create_statement, duckdb_type, load, copy_statement, run_bounded
)
from schema import TABLES


class NativeFixtureTests(unittest.TestCase):
    def test_decimal_types_preserve_native_fraction_scale(self):
        self.assertEqual(duckdb_type("Int64", 100), "DECIMAL(18,2)")
        self.assertEqual(duckdb_type("Int64", 10000), "DECIMAL(18,4)")
        self.assertIn("l_discount DECIMAL(18,4)", create_statement("lineitem"))
        self.assertIn("l_extendedprice DECIMAL(18,2)", create_statement("lineitem"))
        self.assertIn("l_shipdate DATE", create_statement("lineitem"))
        self.assertNotIn("DECIMAL", create_statement("region"))

    def test_converts_original_dbgen_without_fixed_point_scaling(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "lineitem.tbl"
            destination = Path(root) / "native.tsv"
            values = []
            for _, kind, scale in TABLES["lineitem"]:
                values.append(
                    "sample" if kind == "String" else
                    "1994-06-15" if kind == "Date" else
                    "0.06" if scale == 10000 else
                    "123.45" if scale == 100 else "7"
                )
            source.write_text("|".join(values) + "|\n")
            self.assertEqual(convert_native_table(source, destination, "lineitem"), 1)
            row = destination.read_text().splitlines()[1].split("\t")
            self.assertEqual(row[4:8], ["123.45", "123.45", "0.06", "0.06"])
            self.assertEqual(row[10], "1994-06-15")

    def test_native_dbgen_input_is_line_bounded(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "region.tbl"
            source.write_bytes(b"1|" + b"x" * 100 + b"|comment|\n")
            with patch("native_reference_fixture.MAX_LINE", 16):
                with self.assertRaisesRegex(ValueError, "exceeds 8 MiB"):
                    convert_native_table(source, Path(root) / "region.tsv", "region")

    def test_refuses_symlinked_dbgen_input(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "region.tbl"
            source.write_bytes(b"1|EUROPE|comment|\n")
            alias = Path(root) / "alias.tbl"
            try:
                alias.symlink_to(source)
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unavailable")
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                convert_native_table(alias, Path(root) / "region.tsv", "region")


    def test_failures_dont_create_a_partial_reference_database(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "region.tbl").write_text("1|EUROPE|comment|\n")
            target = root / "reference.duckdb"
            def reject(args):
                return CompletedProcess(args, 1, b"", b"failed")
            with self.assertRaisesRegex(RuntimeError, "import failed"):
                load("duckdb", target, root, ["region"], execute=reject)
            self.assertFalse(target.exists())


    def test_native_duckdb_cli_capture_is_spooled_and_bounded(self):
        from types import SimpleNamespace
        from unittest.mock import patch

        def excessive(_cmd, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"x" * (1024 * 1024 + 1))
            return SimpleNamespace(returncode=0)

        with patch("native_reference_fixture.subprocess.run", side_effect=excessive):
            with self.assertRaisesRegex(RuntimeError, "1 MiB"):
                run_bounded(["duckdb"], 30)

        def stderr_failure(_cmd, **kwargs):
            kwargs["stderr"].write(b"x" * 50000 + b"final error")
            return SimpleNamespace(returncode=1)

        with patch("native_reference_fixture.subprocess.run", side_effect=stderr_failure):
            response = run_bounded(["duckdb"], 30)
        self.assertEqual(response.returncode, 1)
        self.assertLessEqual(len(response.stderr), 500)
        self.assertTrue(response.stderr.endswith(b"final error"))

    def test_rejects_bad_selection_and_escapes_sql_filename(self):
        with self.assertRaisesRegex(ValueError, "unique"):
            load("duckdb", "db", ".", ["region", "region"])
        self.assertIn("a''b", copy_statement("region", "a'b"))


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path

from convert_dbgen import convert_directory, convert_table, convert_value
from schema import TABLES


def sample(table):
    fields = []
    for name, kind, scale in TABLES[table]:
        if kind == "String":
            fields.append("text")
        elif kind == "Date":
            fields.append("1994-05-06")
        elif scale == 10000:
            fields.append("0.06")
        elif scale == 100:
            fields.append("123.45")
        else:
            fields.append("1")
    return "|".join(fields) + "|\n"


class DbgenConversionTests(unittest.TestCase):
    def test_all_column_types_and_trailing_pipe(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "lineitem.tbl"
            target = Path(root) / "lineitem.tsv"
            source.write_text(sample("lineitem"), encoding="utf-8")
            self.assertEqual(convert_table(source, target, "lineitem"), 1)
            lines = target.read_text().splitlines()
            self.assertEqual(len(lines), 2)
            cells = lines[1].split("\t")
            self.assertEqual(len(cells), 16)
            self.assertEqual(cells[4:8], ["12345", "12345", "600", "600"])
            self.assertEqual(cells[10], "1994-05-06")

    def test_exact_scaled_values_without_decimal_rounding(self):
        cases = [
            ("123.45", 100, 12345),
            ("-23.50", 100, -2350),
            ("0.07", 10000, 700),
            ("0.0700000", 10000, 700),
            ("-0.0001", 10000, -1),
            ("-0.00", 10000, 0),
            ("7", 10000, 70000),
            ("0.000000", 10000, 0),
        ]
        for raw, scale, expected in cases:
            with self.subTest(raw=raw, scale=scale):
                from convert_dbgen import parse_scaled_number
                self.assertEqual(parse_scaled_number(raw, scale), expected)
        for raw in ("1.00001", "-0.00001", "0.01001"):
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "precision"):
                    from convert_dbgen import parse_scaled_number
                    parse_scaled_number(raw, 10000)

    def test_rejects_precision_and_invalid_rows_without_publishing(self):
        with self.assertRaisesRegex(ValueError, "precision"):
            convert_value("0.00001", "Int64", 10000)
        with self.assertRaisesRegex(ValueError, "range"):
            convert_value("-1", "UInt64", 1)
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "region.tbl").write_text("1|bad|row\n")
            with self.assertRaisesRegex(ValueError, "trailing pipe"):
                convert_table(path / "region.tbl", path / "region.tsv", "region")
            self.assertFalse((path / "region.tsv").exists())

    def test_rejects_ascii_control_bytes_in_string_fields(self):
        for value in ("a\x00b", "a\x1fb", "a\x7fb"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "control-character"):
                    convert_value(value, "String", 1)

    def test_rejects_invalid_utf8_without_publishing(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "region.tbl"
            target = Path(root) / "region.tsv"
            source.write_bytes(b"1|name|comment|\n" + b"2|bad\xff|comment|\n")
            with self.assertRaisesRegex(ValueError, "row 2: invalid UTF-8"):
                convert_table(source, target, "region")
            self.assertFalse(target.exists())
            self.assertFalse(target.with_name("region.tsv.tmp").exists())

    def test_rejects_empty_and_duplicate_table_selection(self):
        for selected in ([], ["region", "region"]):
            with self.subTest(selected=selected):
                with self.assertRaisesRegex(ValueError, "unique"):
                    convert_directory(Path("missing"), Path("output"), selected)

    def test_converts_selected_tables_only(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "nation.tbl").write_text(sample("nation"))
            counts = convert_directory(path, path / "converted", ["nation"])
            self.assertEqual(counts, {"nation": 1})
            self.assertTrue((path / "converted" / "nation.tsv").exists())


if __name__ == "__main__":
    unittest.main()

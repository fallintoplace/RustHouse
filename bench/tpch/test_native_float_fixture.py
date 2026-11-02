import tempfile
import unittest
from pathlib import Path

from convert_dbgen import convert_table, convert_value
from load_dbgen import load_tables
from schema import TABLES, create_sql, names


class UnscaledFloatFixtureTests(unittest.TestCase):
    def test_sql_types_change_only_for_scaled_decimal_columns(self):
        schema = create_sql("lineitem", numeric_mode="float")
        self.assertIn("l_quantity Float64", schema)
        self.assertIn("l_extendedprice Float64", schema)
        self.assertIn("l_discount Float64", schema)
        self.assertIn("l_shipdate Date", schema)
        self.assertIn("l_orderkey UInt64", schema)
        self.assertIn("l_quantity Int64", create_sql("lineitem"))

    def test_exact_source_lexeme_survives_unscaled_conversion(self):
        for raw, scale in [("0.07", 10000), ("-0.0001", 10000),
                           ("123.45", 100), ("0.0700", 10000)]:
            with self.subTest(raw=raw):
                self.assertEqual(convert_value(raw, "Int64", scale, "float"), raw)
        self.assertEqual(convert_value("0.07", "Int64", 10000), "700")
        with self.assertRaisesRegex(ValueError, "precision"):
            convert_value("0.00001", "Int64", 10000, "float")
        with self.assertRaisesRegex(ValueError, "numeric mode"):
            create_sql("lineitem", numeric_mode="invalid")

    def test_converter_and_loader_use_the_same_unscaled_mode(self):
        values = []
        for name, kind, scale in TABLES["lineitem"]:
            if kind == "String":
                values.append("text")
            elif kind == "Date":
                values.append("1994-01-01")
            elif scale == 10000:
                values.append("0.07")
            elif scale == 100:
                values.append("123.45")
            else:
                values.append("1")
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            source = directory / "lineitem.tbl"
            source.write_text("|".join(values) + "|\n", encoding="utf-8")
            target = directory / "lineitem.tsv"
            self.assertEqual(convert_table(source, target, "lineitem", "float"), 1)
            row = target.read_text().splitlines()[1].split("\t")
            self.assertEqual(row[4:8], ["123.45", "123.45", "0.07", "0.07"])
            statements = []
            def execute(sql):
                statements.append(sql)
                return b"1\n" if sql.startswith("SELECT count") else b""
            result = load_tables("rusthouse", directory / "db", directory,
                                 ["lineitem"], execute=execute, numeric_mode="float")
            self.assertEqual(result["tables"][0]["rows"], 1)
            self.assertIn("l_discount Float64", statements[0])


if __name__ == "__main__":
    unittest.main()

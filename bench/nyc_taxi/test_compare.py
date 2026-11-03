"""Unit coverage for the SQL parity runner without a DuckDB dependency."""
import json
import unittest
from pathlib import Path

from bench.nyc_taxi.compare import WORKLOAD, parse_tsv, rows_match, value_matches


class TaxiParityTests(unittest.TestCase):
    def test_pinned_workload_contains_ten_unique_queries(self):
        cases = json.loads(WORKLOAD.read_text(encoding="utf-8"))
        self.assertEqual([item["id"] for item in cases], list(range(1, 11)))
        self.assertTrue(all(item["sql"].strip().endswith(";") for item in cases))
        self.assertEqual(sum("JOIN" in item["sql"] for item in cases), 3)
        self.assertIn("SUM(CASE WHEN", cases[5]["sql"])

    def test_tsv_preserves_null_and_detects_bad_width(self):
        self.assertEqual(parse_tsv("a\tb\n5\t\\N\n"), [("5", None)])
        with self.assertRaisesRegex(ValueError, "field count"):
            parse_tsv("a\tb\n5\n")
        with self.assertRaisesRegex(ValueError, "no TSV header"):
            parse_tsv("")

    def test_row_order_and_numeric_precision(self):
        self.assertEqual(rows_match([("5", "10.0000001")], [(5, 10.0)]), (True, ""))
        self.assertFalse(rows_match([("Brooklyn",), ("Queens",)], [("Queens",), ("Brooklyn",)])[0])
        self.assertFalse(rows_match([("1",)], [(2,)])[0])
        self.assertTrue(value_matches(None, None))
        self.assertFalse(value_matches("10", None))


if __name__ == "__main__":
    unittest.main()

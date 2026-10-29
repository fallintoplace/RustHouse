import csv
import io
import unittest
from report_csv import write_csv, rows_from_report

class SpreadsheetErrorEscapingTests(unittest.TestCase):
    def test_escapes_formula_like_error_messages(self):
        for formula in ("=1+1", "+SUM(1,2)", "-1+2", "@cmd", "  =5", "\t=1"):
            with self.subTest(formula=formula):
                row = rows_from_report({"results": [
                    {"query": 1, "status": "error", "error": formula}]})[0]
                self.assertTrue(row["error"].startswith("'"))
                output = io.StringIO()
                write_csv({"results": [{"query": 1, "status": "error", "error": formula}]}, output)
                self.assertTrue(next(csv.DictReader(io.StringIO(output.getvalue())))["error"].startswith("'"))

    def test_plain_errors_unchanged(self):
        self.assertEqual(rows_from_report({"results": [
            {"query": 1, "status": "error", "error": "unsupported JOIN"}]})[0]["error"],
            "unsupported JOIN")

if __name__ == "__main__":
    unittest.main()

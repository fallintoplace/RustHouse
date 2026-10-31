"""Malformed JSON report fields must not reach CSV output."""
import tempfile
import unittest
from pathlib import Path

from report_csv import load_report


class StrictCsvReportJsonTests(unittest.TestCase):
    def test_rejects_duplicate_result_fields(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_text('{"results":[],"results":[{"query":1}]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                load_report(path)

    def test_rejects_nonstandard_json_numbers(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            for number in ("NaN", "Infinity", "-Infinity", "1e10000", "-1e10000"):
                with self.subTest(number=number):
                    path.write_text('{"results":[],"value":' + number + '}', encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "invalid"):
                        load_report(path)

    def test_valid_report_json_still_loads(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_text('{"results":[{"query":1,"status":"timeout"}]}', encoding="utf-8")
            self.assertEqual(load_report(path)["results"][0]["query"], 1)


if __name__ == "__main__":
    unittest.main()

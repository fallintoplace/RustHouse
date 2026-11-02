"""Malformed JSON report fields must not reach CSV output."""
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from report_csv import load_report, report_metadata_identity


class StrictCsvReportJsonTests(unittest.TestCase):
    def test_windows_uses_birthtime_for_stable_identity(self):
        one = SimpleNamespace(
            st_dev=3, st_ino=4, st_size=20, st_mtime_ns=100,
            st_ctime_ns=101, st_birthtime_ns=300,
        )
        same_file_different_ctime = SimpleNamespace(
            st_dev=3, st_ino=4, st_size=20, st_mtime_ns=100,
            st_ctime_ns=102, st_birthtime_ns=300,
        )
        self.assertEqual(
            report_metadata_identity(one, windows=True),
            report_metadata_identity(same_file_different_ctime, windows=True),
        )
        self.assertNotEqual(
            report_metadata_identity(one, windows=False),
            report_metadata_identity(same_file_different_ctime, windows=False),
        )

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

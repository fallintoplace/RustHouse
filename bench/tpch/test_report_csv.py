import csv
import io
import unittest

from report_csv import rows_from_report, write_csv


class ReportCSVTests(unittest.TestCase):
    def test_exports_verified_timings_without_fake_failure_numbers(self):
        report = {"results": [
            {"query": 6, "status": "passed",
             "timings_seconds": [0.01, 0.02],
             "latency": {"median_seconds": 0.015, "p90_seconds": 0.02},
             "error": None},
            {"query": 12, "status": "timeout",
             "timings_seconds": [0.01],
             "latency": {"median_seconds": 0.01, "p90_seconds": 0.01},
             "error": "slow"},
        ]}
        output = io.StringIO()
        write_csv(report, output)
        rows = list(csv.DictReader(io.StringIO(output.getvalue())))
        self.assertEqual(rows[0]["median_ms"], "15.000000")
        self.assertEqual(rows[0]["samples"], "2")
        self.assertEqual(rows[1]["median_ms"], "")
        self.assertEqual(rows[1]["samples"], "0")
        self.assertEqual(rows[1]["error"], "slow")

    def test_rejects_incomplete_json(self):
        for payload in ({}, {"results": None}, {"results": [{}]}):
            with self.assertRaises(ValueError):
                rows_from_report(payload)


if __name__ == "__main__":
    unittest.main()

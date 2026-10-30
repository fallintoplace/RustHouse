"""Tests for the CI latency report completeness gate."""

import tempfile
import unittest
from pathlib import Path

from bench.common import latency_summary
from bench.latency.run import make_cases
from bench.latency.validate_report import load_report, validate_report


def good_report():
    samples = [0.1, 0.2]
    return {
        "suite": "generated-sql-latency",
        "rows": 512,
        "runs": 2,
        "warmups": 1,
        "results": [
            {
                "id": index,
                "name": case.name,
                "sql": case.sql,
                "status": "passed",
                "seconds": samples.copy(),
                "latency": latency_summary(samples),
                "error": None,
            }
            for index, case in enumerate(make_cases(512), start=1)
        ],
    }


class ValidateLatencyReportTests(unittest.TestCase):
    def verify(self, report):
        return validate_report(report, rows=512, runs=2, warmups=1)

    def test_accepts_twenty_verified_queries(self):
        self.assertEqual(self.verify(good_report()), 20)

    def test_rejects_missing_query(self):
        report = good_report()
        report["results"].pop()
        with self.assertRaisesRegex(ValueError, "exactly 20"):
            self.verify(report)

    def test_rejects_mismatched_query(self):
        report = good_report()
        report["results"][0]["sql"] = "SELECT 1"
        with self.assertRaisesRegex(ValueError, "expected generated SQL"):
            self.verify(report)

    def test_rejects_unverified_result(self):
        report = good_report()
        report["results"][0]["status"] = "mismatch"
        with self.assertRaisesRegex(ValueError, "exact-output"):
            self.verify(report)

    def test_rejects_incomplete_samples(self):
        report = good_report()
        report["results"][0]["seconds"].pop()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            self.verify(report)

    def test_rejects_invalid_samples(self):
        report = good_report()
        report["results"][0]["seconds"][0] = float("nan")
        with self.assertRaisesRegex(ValueError, "invalid timing"):
            self.verify(report)

    def test_rejects_inconsistent_summary(self):
        report = good_report()
        report["results"][0]["latency"]["median_seconds"] = 99
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            self.verify(report)

    def test_rejects_wrong_workload_size(self):
        report = good_report()
        report["rows"] = 1024
        with self.assertRaisesRegex(ValueError, "expected rows"):
            self.verify(report)




class BoundedLatencyReportReadTests(unittest.TestCase):
    def test_rejects_oversized_json_before_parsing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "oversized.json"
            path.write_bytes(b" " * (2 * 1024 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "exceeds 2 MiB"):
                load_report(path)

    def test_loads_small_json(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text('{"suite":"generated-sql-latency"}', encoding="utf-8")
            self.assertEqual(load_report(path)["suite"], "generated-sql-latency")

if __name__ == "__main__":
    unittest.main()

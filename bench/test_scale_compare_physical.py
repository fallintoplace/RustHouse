import copy
import unittest

from bench.scale.compare_physical import compare


def report(median=1.0, rows=1_000_000_000):
    return {"workload": "rusthouse-persisted-wide-part-full-scans",
            "rows_written_and_verified": rows,
            "count_even": {"query": "count", "expected": 10,
                           "median_seconds": median, "verified": True},
            "sum_even": {"query": "sum", "expected": 20,
                         "median_seconds": median, "verified": True}}


class ComparePhysicalTests(unittest.TestCase):
    def test_within_budget(self):
        result = compare(report(), report(1.05), max_slowdown_percent=6)
        self.assertTrue(result["all_within_budget"])
        self.assertEqual(len(result["cases"]), 2)

    def test_reports_regression(self):
        result = compare(report(), report(1.5), max_slowdown_percent=10)
        self.assertFalse(result["all_within_budget"])
        self.assertGreater(result["cases"][0]["slowdown_percent"], 49)

    def test_fails_on_mismatched_rows_and_unverified_results(self):
        with self.assertRaisesRegex(ValueError, "same"):
            compare(report(), report(rows=99))
        candidate = report()
        candidate["count_even"]["verified"] = False
        with self.assertRaisesRegex(ValueError, "verified"):
            compare(report(), candidate)


if __name__ == "__main__":
    unittest.main()

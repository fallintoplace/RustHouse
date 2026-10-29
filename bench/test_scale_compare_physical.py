import copy
import unittest

from bench.scale.compare_physical import compare
from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference


def report(median=1.0, rows=1_000_000_000):
    count, total = even_reference(rows)
    return {"workload": "rusthouse-persisted-wide-part-full-scans",
            "rows_written_and_verified": rows,
            "count_even": {"query": COUNT_SQL, "expected": count,
                           "median_seconds": median, "verified": True},
            "sum_even": {"query": SUM_SQL, "expected": total,
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

    def test_rejects_identically_wrong_oracles_in_both_reports(self):
        fake = report()
        fake["count_even"]["expected"] += 1
        with self.assertRaisesRegex(ValueError, "independent oracle"):
            compare(fake, copy.deepcopy(fake))
        fake = report()
        fake["sum_even"]["query"] = "SELECT sum(id) FROM events"
        with self.assertRaisesRegex(ValueError, "independent oracle"):
            compare(fake, copy.deepcopy(fake))

    def test_fails_on_mismatched_rows_and_unverified_results(self):
        with self.assertRaisesRegex(ValueError, "same"):
            compare(report(), report(rows=99))
        candidate = report()
        candidate["count_even"]["verified"] = False
        with self.assertRaisesRegex(ValueError, "verified"):
            compare(report(), candidate)


if __name__ == "__main__":
    unittest.main()

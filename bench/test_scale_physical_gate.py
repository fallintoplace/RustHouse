import unittest

from bench.scale.physical_gate import validate
from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference


def example(rows=1_000_000_000, median=1.0):
    count, total = even_reference(rows)
    return {"workload": "rusthouse-persisted-wide-part-full-scans",
            "rows_written_and_verified": rows,
            "count_even": {"query": COUNT_SQL, "expected": count, "verified": True,
                           "median_seconds": median, "p90_seconds": median + 0.1},
            "sum_even": {"query": SUM_SQL, "expected": total, "verified": True,
                         "median_seconds": median, "p90_seconds": median + 0.1}}


class PhysicalGateTests(unittest.TestCase):
    def test_passes_verified_1b_snapshots(self):
        self.assertTrue(validate(example(), max_median_seconds=2)["all_passed"])

    def test_fails_timing_target(self):
        self.assertFalse(validate(example(median=4), max_median_seconds=3)["all_passed"])

    def test_rejects_small_and_unverified(self):
        with self.assertRaisesRegex(ValueError, "enough"):
            validate(example(rows=100))
        broken = example()
        broken["count_even"]["verified"] = False
        with self.assertRaisesRegex(ValueError, "verified"):
            validate(broken)

    def test_rejects_incorrect_reference_or_sql(self):
        wrong_sum = example()
        wrong_sum["sum_even"]["expected"] = 0
        with self.assertRaisesRegex(ValueError, "oracle mismatch"):
            validate(wrong_sum)
        wrong_query = example()
        wrong_query["count_even"]["query"] = "SELECT count(*) FROM numbers(10)"
        with self.assertRaisesRegex(ValueError, "oracle mismatch"):
            validate(wrong_query)

    def test_rejects_inconsistent_percentile(self):
        broken = example()
        broken["sum_even"]["p90_seconds"] = 0.1
        with self.assertRaisesRegex(ValueError, "p90"):
            validate(broken)


if __name__ == "__main__":
    unittest.main()

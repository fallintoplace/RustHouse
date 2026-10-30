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

    def test_rejects_float_oracles_even_when_numerically_equal(self):
        invalid = example()
        invalid["count_even"]["expected"] = float(invalid["count_even"]["expected"])
        with self.assertRaisesRegex(ValueError, "oracle mismatch"):
            validate(invalid)

    def test_rejects_inconsistent_percentile(self):
        broken = example()
        broken["sum_even"]["p90_seconds"] = 0.1
        with self.assertRaisesRegex(ValueError, "p90"):
            validate(broken)

    def test_sample_spread_budget_rejects_noisy_measurements(self):
        report = example()
        with self.assertRaisesRegex(ValueError, "requires timing samples"):
            validate(report, max_sample_spread_percent=20)
        for key in ("count_even", "sum_even"):
            report[key]["samples_seconds"] = [0.85, 1.15]
            report[key]["p90_seconds"] = 1.15
        strict = validate(report, max_sample_spread_percent=20)
        self.assertFalse(strict["all_passed"])
        self.assertAlmostEqual(strict["checks"][0]["sample_spread_percent"], 30)
        relaxed = validate(report, max_sample_spread_percent=35)
        self.assertTrue(relaxed["all_passed"])
        with self.assertRaisesRegex(ValueError, "max_sample_spread_percent"):
            validate(report, max_sample_spread_percent=float("nan"))

    def test_rejects_forged_summary_even_without_spread_budget(self):
        report = example()
        for key in ("count_even", "sum_even"):
            report[key]["samples_seconds"] = [0.8, 1.2]
        with self.assertRaisesRegex(ValueError, "summary differs"):
            validate(report)
        for key in ("count_even", "sum_even"):
            report[key]["p90_seconds"] = 1.2
        self.assertTrue(validate(report)["all_passed"])

    def test_rejects_corrupt_timing_samples(self):
        broken = example()
        broken["runs"] = 2
        broken["count_even"]["samples_seconds"] = [1.0, float("nan")]
        with self.assertRaisesRegex(ValueError, "samples"):
            validate(broken)
        broken["count_even"]["samples_seconds"] = [1.0]
        with self.assertRaisesRegex(ValueError, "sample count"):
            validate(broken)




class LargeFinitePhysicalSpreadTests(unittest.TestCase):
    def test_large_finite_durations_do_not_overflow_spread(self):
        report = example(median=1.5e307)
        for name in ("count_even", "sum_even"):
            report[name]["samples_seconds"] = [1e307, 2e307]
            report[name]["p90_seconds"] = 2e307
        result = validate(report, max_sample_spread_percent=100)
        self.assertTrue(result["all_passed"])
        self.assertAlmostEqual(result["checks"][0]["sample_spread_percent"], 66.6666666667)

if __name__ == "__main__":
    unittest.main()

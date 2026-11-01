"""Reject invalid scan timings without exceptions or false median failures."""
import unittest

from bench.scale.physical_gate import validate
from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference


def report(samples):
    rows = 10
    count, total = even_reference(rows)
    return {"workload": "rusthouse-persisted-wide-part-full-scans",
            "rows_written_and_verified": rows,
            "runs": len(samples),
            "count_even": {"verified": True, "query": COUNT_SQL, "expected": count,
                           "median_seconds": samples[len(samples) // 2],
                           "p90_seconds": max(samples), "samples_seconds": samples},
            "sum_even": {"verified": True, "query": SUM_SQL, "expected": total,
                         "median_seconds": samples[len(samples) // 2],
                         "p90_seconds": max(samples), "samples_seconds": samples}}


class PhysicalScanExtremeTimingTests(unittest.TestCase):
    def test_extreme_integer_sample_fails_validation_without_overflow(self):
        for samples in ([10 ** 400], [0.1, 10 ** 400], [True], [-1], [float("inf")]):
            with self.subTest(value=str(samples)[:40]), self.assertRaises(ValueError):
                validate(report(samples), min_rows=10)

    def test_extreme_timing_target_rejected(self):
        for limit in (10 ** 400, 1e309):
            with self.assertRaisesRegex(ValueError, "timing targets"):
                validate(report([1]), min_rows=10, max_median_seconds=limit)

    def test_even_median_does_not_overflow_finite_samples(self):
        result = report([1e308, 1e308])
        for name in ("count_even", "sum_even"):
            result[name]["median_seconds"] = 1e308
        verdict = validate(result, min_rows=10)
        self.assertTrue(verdict["all_passed"])
        self.assertEqual(verdict["checks"][0]["median_seconds"], 1e308)


if __name__ == "__main__":
    unittest.main()

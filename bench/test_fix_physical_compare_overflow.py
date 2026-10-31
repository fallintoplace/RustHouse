"""Explicit errors replace nonfinite physical scan comparison output."""
import unittest

from bench.scale.compare_physical import compare
from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference


def report(median):
    count, total = even_reference(10)
    return {
        "workload": "rusthouse-persisted-wide-part-full-scans",
        "rows_written_and_verified": 10,
        "count_even": {"query": COUNT_SQL, "expected": count, "verified": True,
                       "median_seconds": median},
        "sum_even": {"query": SUM_SQL, "expected": total, "verified": True,
                     "median_seconds": median},
    }


class PhysicalCompareOverflowTests(unittest.TestCase):
    def test_extreme_finite_medians_cannot_emit_infinity(self):
        with self.assertRaisesRegex(ValueError, "median slowdown ratio overflows"):
            compare(report(1e-300), report(1e300))

    def test_extreme_finite_p90_values_cannot_emit_infinity(self):
        old = report(1e-300)
        new = report(1e-300)
        for field in ("count_even", "sum_even"):
            old[field]["p90_seconds"] = 1e-300
            new[field]["p90_seconds"] = 1e300
        with self.assertRaisesRegex(ValueError, "p90 slowdown ratio overflows"):
            compare(old, new)


if __name__ == "__main__":
    unittest.main()

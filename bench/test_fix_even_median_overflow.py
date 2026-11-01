"""Even-sized latency samples must remain finite when their inputs are finite."""
import math
import unittest

from bench.common import latency_summary


class FiniteMedianTests(unittest.TestCase):
    def test_two_large_finite_samples_do_not_overflow(self):
        for samples in ([1e308, 1e308], [1e307, 1e308]):
            with self.subTest(samples=samples):
                summary = latency_summary(samples)
                self.assertTrue(math.isfinite(summary["median_seconds"]))
                self.assertLessEqual(summary["min_seconds"], summary["median_seconds"])
                self.assertLessEqual(summary["median_seconds"], summary["max_seconds"])

    def test_small_and_regular_even_samples_remain_exact(self):
        self.assertEqual(latency_summary([1.0, 3.0])["median_seconds"], 2.0)
        self.assertEqual(latency_summary([1e-300, 1e-300])["median_seconds"], 1e-300)


if __name__ == "__main__":
    unittest.main()

import unittest

from bench.release_gate import safe_median, verify_latency_baseline


def report(samples):
    return {
        "reference_checked": True,
        "runs": len(samples),
        "results": [{
            "query": 1, "status": "passed", "timings_seconds": samples,
        }],
    }


class ExtremeLatencyReleaseGateTests(unittest.TestCase):
    def test_large_even_median_remains_finite(self):
        self.assertEqual(safe_median([1e308, 1e308]), 1e308)
        self.assertEqual(safe_median([1, 3, 5, 7]), 4)

    def test_large_finite_regression_cannot_escape_threshold(self):
        with self.assertRaisesRegex(ValueError, "slowdown"):
            verify_latency_baseline(report([1.3e308]), report([1e308]), {1})

    def test_small_finite_regression_is_detected(self):
        with self.assertRaisesRegex(ValueError, "slowdown"):
            verify_latency_baseline(report([1.3e-307]), report([1e-307]), {1})
        verify_latency_baseline(report([1.2e-307]), report([1e-307]), {1})


if __name__ == "__main__":
    unittest.main()

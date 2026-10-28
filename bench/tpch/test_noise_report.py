import unittest
from noise_report import assess_query, assess_report


class NoiseReportTests(unittest.TestCase):
    def test_marks_repeatable_verified_samples_as_stable(self):
        result = assess_query({"query": 6, "status": "passed",
                               "timings_seconds": [1, 1, 1, 1, 1]})
        self.assertEqual(result["state"], "stable")
        self.assertEqual(result["cv_percent"], 0)
        self.assertEqual(result["mad_seconds"], 0)

    def test_marks_noisy_and_insufficient_measurements(self):
        report = assess_report({"results": [
            {"query": 6, "status": "passed", "timings_seconds": [1, 2, 3, 4, 5]},
            {"query": 1, "status": "passed", "timings_seconds": [1, 2]},
            {"query": 3, "status": "unverified", "timings_seconds": [1] * 5},
        ]})
        self.assertEqual([q["state"] for q in report["queries"]],
                         ["noisy", "insufficient_samples", "not_verified"])

    def test_rejects_invalid_thresholds_and_samples(self):
        with self.assertRaises(ValueError):
            assess_query({"query": 1}, min_samples=1)
        invalid = assess_query({"query": 1, "status": "passed",
                                "timings_seconds": [float("nan")] * 5})
        self.assertEqual(invalid["state"], "invalid_samples")
        for values in ([0] * 5, [True] * 5, [1, 1, 1, 0, 1]):
            result = assess_query({"query": 6, "status": "passed",
                                   "timings_seconds": values})
            self.assertEqual(result["state"], "invalid_samples")


if __name__ == "__main__":
    unittest.main()

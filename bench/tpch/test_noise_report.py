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

    def test_zero_negative_and_boolean_timings_are_invalid(self):
        for bad in (0.0, -0.1, True, float("inf")):
            with self.subTest(sample=bad):
                row = assess_query({
                    "query": 6,
                    "status": "passed",
                    "timings_seconds": [1.0, 1.0, bad, 1.0, 1.0],
                })
                self.assertEqual(row["state"], "invalid_samples")

    def test_rejects_empty_reports_that_would_bypass_the_gate(self):
        with self.assertRaisesRegex(ValueError, "invalid TPC-H report"):
            assess_report({"results": []})

    def test_rejects_fractional_sample_counts_and_boolean_thresholds(self):
        for samples in (True, 2.5):
            with self.assertRaisesRegex(ValueError, "thresholds"):
                assess_query({"query": 1}, min_samples=samples)
        for threshold in (True, float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "thresholds"):
                assess_query({"query": 1}, max_cv_percent=threshold)

    def test_rejects_invalid_thresholds_and_samples(self):
        with self.assertRaises(ValueError):
            assess_query({"query": 1}, min_samples=1)
        invalid = assess_query({"query": 1, "status": "passed",
                                "timings_seconds": [float("nan")] * 5})
        self.assertEqual(invalid["state"], "invalid_samples")


class NoiseReportQueryIdTests(unittest.TestCase):
    def test_rejects_duplicate_and_invalid_query_identifiers(self):
        for results in (
            [{"query": 6, "status": "passed"}, {"query": 6, "status": "passed"}],
            [{"query": True, "status": "passed"}],
            [{"query": "6", "status": "passed"}],
            [None],
        ):
            with self.subTest(results=results):
                with self.assertRaisesRegex(ValueError, "distinct positive"):
                    assess_report({"results": results})

if __name__ == "__main__":
    unittest.main()

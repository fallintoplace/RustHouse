import unittest
from noise_report import assess_query, assess_report

class MalformedNoiseTests(unittest.TestCase):
    def test_rejects_string_sample_payload(self):
        result = assess_query({"query": 1, "status": "passed", "timings_seconds": "11111"})
        self.assertEqual(result["state"], "invalid_samples")

    def test_rejects_duplicate_queries(self):
        report = {"results": [{"query": 1, "status": "passed", "timings_seconds": [1] * 5},
                              {"query": 1, "status": "passed", "timings_seconds": [2] * 5}]}
        with self.assertRaisesRegex(ValueError, "duplicate"):
            assess_report(report)

    def test_preserves_valid_results(self):
        result = assess_report({"results": [{"query": 1, "status": "passed",
                                             "timings_seconds": [1] * 5}]})
        self.assertEqual(result["queries"][0]["state"], "stable")

if __name__ == "__main__":
    unittest.main()

import unittest
from bench.compare import compare, elapsed


def result(number, status, seconds):
    return {"query": number, "status": status, "latency": {"median_seconds": seconds}}


class BenchmarkComparisonTests(unittest.TestCase):
    def test_detects_slowdowns_and_speedups(self):
        baseline = {"engine": "rusthouse-cli", "rows": 100,
                    "results": [result(1, "passed", 0.01), result(2, "passed", 0.02)]}
        candidate = {"engine": "rusthouse-cli", "rows": 100,
                     "results": [result(1, "passed", 0.013), result(2, "passed", 0.015)]}
        report = compare(baseline, candidate, slowdown_threshold_percent=10)
        self.assertEqual(report["compared_queries"], 2)
        self.assertEqual([x["query"] for x in report["regressions"]], [1])
        self.assertEqual([x["query"] for x in report["improvements"]], [2])

    def test_correctness_downgrade_is_always_reported(self):
        before = {"results": [result(6, "passed", 0.01)]}
        after = {"results": [result(6, "unverified", 0.001)]}
        report = compare(before, after, slowdown_threshold_percent=100)
        self.assertEqual(report["regressions"][0]["reason"], "correctness_downgrade")
        self.assertEqual(report["compared_queries"], 0)

    def test_rejects_different_scales_and_query_sets(self):
        one = {"rows": 100, "results": [result(1, "passed", 0.01)]}
        other = {"rows": 200, "results": [result(1, "passed", 0.01)]}
        with self.assertRaisesRegex(ValueError, "rows"):
            compare(one, other)
        other["rows"] = 100
        other["results"] = [result(2, "passed", 0.01)]
        with self.assertRaisesRegex(ValueError, "query ID sets"):
            compare(one, other)

    def test_rejects_missing_metadata_and_different_sql_fingerprints(self):
        baseline = {"engine": "rusthouse-cli", "query_sha256": "abc",
                    "results": [result(1, "passed", 0.01)]}
        candidate = {"engine": "rusthouse-cli", "query_sha256": "def",
                     "results": [result(1, "passed", 0.01)]}
        with self.assertRaisesRegex(ValueError, "query_sha256"):
            compare(baseline, candidate)
        candidate["query_sha256"] = "abc"
        del candidate["engine"]
        with self.assertRaisesRegex(ValueError, "engine"):
            compare(baseline, candidate)

    def test_unverified_baseline_does_not_produce_latency_claims(self):
        baseline = {"results": [result(1, "unverified", 0.01)]}
        candidate = {"results": [result(1, "passed", 0.001)]}
        report = compare(baseline, candidate)
        self.assertEqual(report["compared_queries"], 0)
        self.assertFalse(report["improvements"])
        self.assertEqual(report["incomparable"][0]["reason"], "query_not_successful")

    def test_invalid_medians_are_not_measured(self):
        self.assertIsNone(elapsed(result(1, "passed", 0)))
        report = compare({"results": [result(1, "passed", 0)]},
                         {"results": [result(1, "passed", 1)]})
        self.assertEqual(report["compared_queries"], 0)
        self.assertEqual(report["incomparable"][0]["reason"], "missing_positive_median")


    def test_rejects_nonobject_reports(self):
        valid = {"results": [result(1, "passed", 0.1)]}
        for malformed in ([], [valid], 2, None, "JSON"):
            with self.subTest(malformed=malformed):
                with self.assertRaisesRegex(ValueError, "JSON objects"):
                    compare(malformed, valid)
                with self.assertRaisesRegex(ValueError, "JSON objects"):
                    compare(valid, malformed)


if __name__ == "__main__":
    unittest.main()

"""Regression tests for comparable SQL release baseline latencies."""
import unittest

from bench.release_gate import verify_latency_baseline


def report(seconds):
    return {
        "reference_checked": True,
        "runs": 2,
        "engine": "rusthouse-cli",
        "workload": "clickbench",
        "rows": 100,
        "timing_scope": "full-query",
        "query_sha256": "same-queries",
        "results": [{"query": 1, "status": "passed",
                     "timings_seconds": [seconds, seconds]}],
    }


class ReleaseLatencyMetadataTests(unittest.TestCase):
    def test_matching_metadata_accepts_baseline(self):
        self.assertIsNone(verify_latency_baseline(report(1.1), report(1.0), {1}))

    def test_rejects_mismatched_identity_before_latency_comparison(self):
        for field, replacement in (("engine", "other"),
                                   ("workload", "other"),
                                   ("rows", 200),
                                   ("timing_scope", "scan-only"),
                                   ("query_sha256", "different-queries")):
            with self.subTest(field=field):
                old, new = report(1.0), report(1.1)
                old[field] = replacement
                with self.assertRaisesRegex(ValueError, field):
                    verify_latency_baseline(new, old, {1})

    def test_rejects_mismatched_metadata_types(self):
        old, new = report(1.0), report(1.1)
        new["rows"] = True
        with self.assertRaisesRegex(ValueError, "rows"):
            verify_latency_baseline(new, old, {1})


if __name__ == "__main__":
    unittest.main()

"""Untrusted benchmark manifests must report invalid samples, not crash."""
import json
import tempfile
import unittest
from pathlib import Path

from bench.release_gate import load_manifest, verify, verify_latency_baseline


def report(samples):
    return {"reference_checked": True, "runs": len(samples),
            "results": [{"query": 1, "status": "passed",
                         "timings_seconds": samples}]}


class HugeTimingSampleTests(unittest.TestCase):
    def test_rejects_extremely_large_integer_without_overflow(self):
        huge = 10 ** 400
        for samples in ([huge], [0.01, huge], [True], [-1], [float("inf")]):
            with self.subTest(samples=samples), self.assertRaisesRegex(ValueError, "invalid timing samples"):
                verify(report(samples), {1})

    def test_loads_huge_json_integer_but_rejects_during_validation(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "manifest.json"
            path.write_text(json.dumps(report([10 ** 400])), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "invalid timing samples"):
                verify(load_manifest(path), {1})

    def test_latency_baseline_rejects_bad_samples_before_ratio_math(self):
        with self.assertRaisesRegex(ValueError, "invalid timing samples"):
            verify_latency_baseline(report([10 ** 400]), report([1]), {1})

    def test_valid_extreme_floats_still_work(self):
        self.assertEqual(verify(report([1e308, 1e-307]), {1}), 1)


if __name__ == "__main__":
    unittest.main()

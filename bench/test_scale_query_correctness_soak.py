import hashlib
import tempfile
import unittest
from pathlib import Path

from bench.scale.query_correctness_soak import digest_file, percentile, soak


class CorrectnessSoakTests(unittest.TestCase):
    def test_digest_and_nearest_rank_percentile(self):
        with tempfile.TemporaryFile() as source:
            source.write(b"count\n2\n")
            source.seek(0)
            self.assertEqual(digest_file(source),
                             (hashlib.sha256(b"count\n2\n").hexdigest(), 8))
        self.assertEqual(percentile([3, 1, 2], 95), 3)

    def test_matches_each_iteration_and_rejects_drift(self):
        with tempfile.TemporaryDirectory() as temp:
            oracle = Path(temp) / "expected.tsv"
            oracle.write_bytes(b"count\n7\n")
            digest = hashlib.sha256(oracle.read_bytes()).hexdigest()
            checks = iter([(digest, 8, 0.1), (digest, 8, 0.2),
                           (digest, 8, 0.3)])
            report = soak("x", "x", "select", oracle, runs=2, warmups=1,
                          execute=lambda: next(checks))
            self.assertEqual(report["p50_seconds"], 0.25)
            with self.assertRaisesRegex(ValueError, "drift"):
                soak("x", "x", "select", oracle, runs=1, warmups=0,
                     execute=lambda: ("0" * 64, 8, 0.1))

    def test_rejects_invalid_run_counts(self):
        with self.assertRaises(ValueError):
            soak("x", "x", "x", "none", runs=0)

    def test_rejects_accidental_unbounded_full_table_soak(self):
        with self.assertRaisesRegex(ValueError, "1000"):
            soak("x", "x", "select", "none", runs=1000, warmups=1)


if __name__ == "__main__":
    unittest.main()

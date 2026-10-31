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

    def test_percentile_rejects_invalid_duration_samples(self):
        for values in ([float("nan")], [float("inf")], [-1], [True], ["0.1"]):
            with self.subTest(values=values), self.assertRaisesRegex(ValueError, "invalid sample"):
                percentile(values, 95)
        self.assertEqual(percentile([0.0, 0.1], 95), 0.1)

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

    def test_soak_subprocess_spools_and_bounds_stderr(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from bench.scale.query_correctness_soak import run_sql

        def fail(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            self.assertNotEqual(kwargs["stderr"], __import__("subprocess").PIPE)
            kwargs["stderr"].write(b"x" * 10_000 + b"last error")
            return SimpleNamespace(returncode=1)

        with patch("bench.scale.query_correctness_soak.subprocess.run", side_effect=fail):
            with self.assertRaises(RuntimeError) as caught:
                run_sql("rusthouse", "db", "SELECT 1", 1)
        self.assertIn("last error", str(caught.exception))
        self.assertLessEqual(len(str(caught.exception)), 500)

    def test_refuses_symlinked_expected_output_before_query(self):
        with tempfile.TemporaryDirectory() as root:
            original = Path(root) / "reference.tsv"
            original.write_bytes(b"count\\n1\\n")
            alias = Path(root) / "reference-link.tsv"
            alias.symlink_to(original)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                soak("rusthouse", "db", "SELECT 1", alias,
                     runs=1, warmups=0, execute=lambda: self.fail("must not query"))

    def test_rejects_bad_elapsed_samples(self):
        with tempfile.TemporaryDirectory() as root:
            oracle = Path(root) / "oracle.tsv"
            oracle.write_bytes(b"data")
            digest = hashlib.sha256(b"data").hexdigest()
            for seconds in (True, "0.1", float("nan"), -1, 10 ** 400):
                with self.subTest(seconds=seconds), self.assertRaisesRegex(ValueError, "elapsed"):
                    soak("x", "db", "SELECT 1", oracle, runs=1, warmups=0,
                         execute=lambda: (digest, 4, seconds))

    def test_rejects_invalid_run_counts(self):
        with self.assertRaises(ValueError):
            soak("x", "x", "x", "none", runs=0)

    def test_rejects_accidental_unbounded_full_table_soak(self):
        with self.assertRaisesRegex(ValueError, "1000"):
            soak("x", "x", "select", "none", runs=1000, warmups=1)


    def test_rejects_mutating_or_multiple_sql_before_launch(self):
        for statement in ("DROP TABLE t", "UPDATE t SET n=1",
                          "SELECT 1; DROP TABLE t", "", "WITH x AS (SELECT 1) DELETE FROM t"):
            with self.subTest(statement=statement), self.assertRaisesRegex(ValueError, "read-only SELECT"):
                soak("rusthouse", "db", statement, "nonexistent-reference.tsv",
                     execute=lambda: self.fail("must not run unsafe SQL"))

if __name__ == "__main__":
    unittest.main()

import subprocess
import tempfile
import unittest
from pathlib import Path

from run import benchmark_one, load_queries, summary

class RunnerTests(unittest.TestCase):
    def test_query_discovery_is_numeric(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "q10.sql").write_text("-- sample\nSELECT 10;\n")
            (directory / "q02.sql").write_text("SELECT 2;\n")
            self.assertEqual(
                load_queries(directory), [(2, "SELECT 2"), (10, "SELECT 10")]
            )

    def test_rejects_zero_query_ids(self):
        for filename in ("q0.sql", "q00.sql"):
            with tempfile.TemporaryDirectory() as root:
                (Path(root) / filename).write_text("SELECT 0;\n")
                with self.assertRaisesRegex(ValueError, "query IDs must be positive"):
                    load_queries(Path(root))

    def test_correctness_is_checked_before_timing(self):
        passed = benchmark_one(6, "SELECT 1", lambda _: b"1\n", 3, 1, b"1\n")
        self.assertEqual(passed["status"], "passed")
        self.assertEqual(len(passed["timings_seconds"]), 3)
        mismatched = benchmark_one(6, "SELECT 1", lambda _: b"2\n", 3, 1, b"1\n")
        self.assertEqual(mismatched["status"], "mismatch")
        self.assertEqual(mismatched["timings_seconds"], [])

    def test_error_and_timeout(self):
        def broken(_):
            raise RuntimeError("unsupported statement")
        self.assertEqual(benchmark_one(1, "SELECT 1", broken, 2, 0)["status"], "error")
        def timed_out(_):
            raise subprocess.TimeoutExpired("rusthouse", 1)
        self.assertEqual(benchmark_one(1, "SELECT 1", timed_out, 2, 0)["status"], "timeout")

    def test_unverified_and_p90(self):
        result = benchmark_one(1, "SELECT 1", lambda _: b"ok\n", 2, 0)
        self.assertEqual(result["status"], "unverified")
        self.assertEqual(summary([1, 4, 2, 3, 5])["p90_seconds"], 5)

if __name__ == "__main__":
    unittest.main()

import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from throughput import run_throughput


class ThroughputTests(unittest.TestCase):
    def test_verified_concurrent_results(self):
        report = run_throughput(
            [(1, "SELECT 1"), (2, "SELECT 2")],
            lambda sql: (sql.split()[-1] + "\n").encode(),
            clients=2, rounds=3, references={1: b"1\n", 2: b"2\n"},
        )
        self.assertEqual(report["planned_executions"], 6)
        self.assertEqual(report["verified_executions"], 6)
        self.assertGreater(report["successful_queries_per_second"], 0)
        self.assertFalse(report["official_tpch_score"])

    def test_only_byte_results_count_as_success(self):
        report = run_throughput(
            [(1, "SELECT 1")], lambda _: "1\\n", rounds=1
        )
        self.assertEqual(report["successful_executions"], 0)
        self.assertEqual(report["results"][0]["status"], "error")
        self.assertIn("bytes", report["results"][0]["error"])
        with self.assertRaisesRegex(ValueError, "references"):
            run_throughput([(1, "SELECT 1")], lambda _: b"1\\n",
                           rounds=1, references={1: "1\\n"})

    def test_mismatch_does_not_count_as_success(self):
        report = run_throughput(
            [(1, "SELECT 1")], lambda _: b"wrong\n",
            clients=1, rounds=1, references={1: b"1\n"},
        )
        self.assertEqual(report["results"][0]["status"], "mismatch")
        self.assertEqual(report["successful_executions"], 0)

    def test_unverified_not_counted_as_verified(self):
        result = run_throughput([(6, "SELECT 6")], lambda _: b"6\n", rounds=1)
        self.assertEqual(result["verified_executions"], 0)
        self.assertEqual(result["successful_executions"], 1)

    def test_verified_mode_rejects_missing_or_partial_goldens(self):
        with self.assertRaisesRegex(ValueError, "reference"):
            run_throughput([(1, "SELECT 1")], lambda _: b"1\\n",
                           require_verified=True)
        with self.assertRaisesRegex(ValueError, "reference"):
            run_throughput([(1, "SELECT 1"), (2, "SELECT 2")],
                           lambda _: b"1\\n", references={1: b"1\\n"},
                           require_verified=True)
        report = run_throughput(
            [(1, "SELECT 1")], lambda _: b"1\\n",
            rounds=1, references={1: b"1\\n"}, require_verified=True,
        )
        self.assertTrue(report["all_executions_verified"])
        self.assertTrue(report["verification_required"])

    def test_limits_are_enforced(self):
        with self.assertRaisesRegex(ValueError, "clients"):
            run_throughput([(1, "sql")], lambda _: b"", clients=0)
        with self.assertRaisesRegex(ValueError, "2200"):
            run_throughput([(i, "sql") for i in range(23)], lambda _: b"",
                           clients=2, rounds=100)


if __name__ == "__main__":
    unittest.main()

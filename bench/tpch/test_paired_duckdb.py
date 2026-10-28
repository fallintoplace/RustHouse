import tempfile
import unittest
from pathlib import Path

from bench.tpch.paired_duckdb import ABBA, BAAB, benchmark_pair, command, compare


class PairedDuckDbTests(unittest.TestCase):
    def test_commands_do_not_use_a_shell_and_keep_sql_as_one_argument(self):
        sql = "SELECT 'a b' AS value"
        rust = command("rusthouse", "rusthouse", "/tmp/rh", sql)
        duck = command("duckdb", "duckdb", "/tmp/ddb", sql)
        self.assertEqual(rust[2], sql)
        self.assertEqual(duck[-1], sql)
        self.assertIn("-readonly", duck)
        self.assertIn("-no-init", duck)
        with self.assertRaises(ValueError):
            command("unknown", "binary", "db", sql)

    def test_paired_samples_use_abba_order_and_check_warmups(self):
        calls = []
        def execute(engine, sql):
            calls.append(engine)
            return b"1\n", {"rusthouse": 0.002, "duckdb": 0.001}[engine]
        result = benchmark_pair("SELECT 1", b"1\n", execute, runs=2, warmups=1)
        self.assertEqual(calls, ["rusthouse", "duckdb", *ABBA, *BAAB])
        self.assertEqual(result["duckdb_over_rusthouse"], 0.5)
        self.assertEqual(len(result["rusthouse_samples_seconds"]), 4)
        self.assertEqual(len(result["duckdb_samples_seconds"]), 4)

    def test_any_mismatch_invalidates_the_speed_comparison(self):
        with self.assertRaisesRegex(ValueError, "differs"):
            benchmark_pair("SELECT 1", b"1\n",
                           lambda engine, sql: (b"2\n", 0.001), 1, 0)
        with self.assertRaisesRegex(ValueError, "timings"):
            benchmark_pair("SELECT 1", b"1\n",
                           lambda engine, sql: (b"1\n", float("nan")), 1, 0)

    def test_ratio_overflow_does_not_create_nonstandard_json(self):
        def extreme(engine, _sql):
            return b"1\n", 1e-320 if engine == "rusthouse" else 1e100

        with self.assertRaisesRegex(ValueError, "ratio"):
            benchmark_pair("SELECT 1", b"1\n", extreme, runs=1, warmups=0)

    def test_unordered_results_preserve_duplicate_counts(self):
        reference = b"A\nB\nB\n"
        def shuffled(engine, statement):
            return b"B\nA\nB\n", 0.001
        result = benchmark_pair(
            "SELECT label FROM t", reference, shuffled,
            runs=1, warmups=0, comparison="unordered",
        )
        self.assertEqual(result["duckdb_over_rusthouse"], 1.0)
        with self.assertRaisesRegex(ValueError, "differs"):
            benchmark_pair("SELECT label FROM t", reference, shuffled, 1, 0)
        with self.assertRaisesRegex(ValueError, "differs"):
            benchmark_pair("SELECT label FROM t", reference,
                           lambda engine, statement: (b"B\nA\nA\n", 0.001),
                           1, 0, comparison="unordered")

    def test_missing_reference_never_claims_speedup(self):
        with tempfile.TemporaryDirectory() as root:
            report = compare("unused", "db", "duckdb", "ddb",
                             [(1, "SELECT 1")], Path(root), 1, 0, 10)
        self.assertFalse(report["all_queries_verified"])
        self.assertIsNone(report["speedup_geomean_duckdb_over_rusthouse"])
        self.assertEqual(report["results"][0]["status"], "error")


if __name__ == "__main__":
    unittest.main()

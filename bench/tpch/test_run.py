import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from run import benchmark_one, compare_reference, load_queries, official_22_coverage, query_fingerprint, select_queries, summary, summarize_coverage

class RunnerTests(unittest.TestCase):
    def test_query_discovery_is_numeric(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "q10.sql").write_text("-- sample\nSELECT 10;\n")
            (directory / "q02.sql").write_text("SELECT 2;\n")
            self.assertEqual(
                load_queries(directory), [(2, "SELECT 2"), (10, "SELECT 10")]
            )

    def test_query_discovery_accepts_uppercase_names(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "Q03.SQL").write_text("SELECT 3;\n")
            (directory / "q01.sql").write_text("SELECT 1;\n")
            self.assertEqual(load_queries(directory), [(1, "SELECT 1"), (3, "SELECT 3")])

    def test_rejects_zero_query_ids(self):
        for filename in ("q0.sql", "q00.sql"):
            with tempfile.TemporaryDirectory() as root:
                (Path(root) / filename).write_text("SELECT 0;\n")
                with self.assertRaisesRegex(ValueError, "query IDs must be positive"):
                    load_queries(Path(root))

    def test_query_selection_uses_loaded_ids(self):
        queries = [(2, "SELECT 2"), (10, "SELECT 10")]
        self.assertEqual(select_queries(queries, "10"), [(10, "SELECT 10")])
        self.assertEqual(select_queries(queries, None), queries)
        with self.assertRaisesRegex(ValueError, "unavailable"):
            select_queries(queries, "1")
        with self.assertRaisesRegex(ValueError, "query IDs or ranges"):
            select_queries(queries, "invalid")

    def test_correctness_is_checked_before_timing(self):
        passed = benchmark_one(6, "SELECT 1", lambda _: b"1\n", 3, 1, b"1\n")
        self.assertEqual(passed["status"], "passed")
        self.assertEqual(len(passed["timings_seconds"]), 3)
        mismatched = benchmark_one(6, "SELECT 1", lambda _: b"2\n", 3, 1, b"1\n")
        self.assertEqual(mismatched["status"], "mismatch")
        self.assertEqual(mismatched["timings_seconds"], [])

    def test_verification_time_does_not_inflate_tpch_latency(self):
        clock = [20.0]
        def execute(_sql):
            clock[0] += 0.04
            return b"ok\n"
        def validate(_actual, _reference, _mode):
            clock[0] += 8.0
            return True
        with mock.patch("run.time.perf_counter", side_effect=lambda: clock[0]), \
             mock.patch("run.compare_reference", side_effect=validate):
            item = benchmark_one(6, "SELECT 1", execute, 1, 0, b"ok\\n")
        self.assertEqual(item["status"], "passed")
        self.assertAlmostEqual(item["timings_seconds"][0], 0.04, places=6)

    def test_unverified_results_are_not_counted_as_correct(self):
        result = benchmark_one(6, "SELECT 6", lambda _: b"6\\n", 2, 0)
        self.assertEqual(result["status"], "unverified")
        self.assertEqual(summarize_coverage([result])["verified_queries"], 0)

    def test_unordered_rows_preserve_duplicate_counts(self):
        expected = b"1\n2\n2\n"
        self.assertFalse(compare_reference(b"2\n1\n2\n", expected))
        self.assertTrue(compare_reference(b"2\n1\n2\n", expected, "unordered"))
        self.assertFalse(compare_reference(b"2\n1\n1\n", expected, "unordered"))
        result = benchmark_one(1, "SELECT id", lambda _: b"2\n1\n2\n",
                               2, 0, expected, comparison="unordered")
        self.assertEqual(result["status"], "passed")

    def test_failed_late_iteration_discards_partial_samples(self):
        calls = 0
        def flaky(_):
            nonlocal calls
            calls += 1
            return b"good\\n" if calls == 1 else b"bad\\n"
        result = benchmark_one(6, "SELECT 1", flaky, 3, 0, b"good\\n")
        self.assertEqual(result["status"], "mismatch")
        self.assertEqual(result["timings_seconds"], [])
        self.assertIsNone(result["latency"])

    def test_select_query_ranges_and_reject_gaps(self):
        queries = [(number, f"SELECT {number}") for number in range(1, 23)]
        result = select_queries(queries, " 1, 6, 10-12 ")
        self.assertEqual([number for number, _ in result], [1, 6, 10, 11, 12])
        for spec in ("", "1-", "5-3", "1,,2", "1-200", "-1"):
            with self.subTest(spec=spec):
                with self.assertRaises(ValueError):
                    select_queries(queries, spec)
        with self.assertRaisesRegex(ValueError, "unavailable"):
            select_queries([(1, "SELECT 1")], "1-2")

    def test_official_22_denominator_is_not_selected_query_count(self):
        few = official_22_coverage([(1, "SELECT 1"), (6, "SELECT 6")])
        self.assertEqual(few["available_queries"], 2)
        self.assertEqual(len(few["missing_query_ids"]), 20)
        self.assertEqual(few["coverage_percent"], 9.09)
        complete = official_22_coverage([(i, "SELECT 1") for i in range(1, 23)])
        self.assertEqual(complete["coverage_percent"], 100.0)
        self.assertEqual(complete["unexpected_query_ids"], [])

    def test_query_fingerprint_stable_and_sensitive_to_sql(self):
        first = [(6, "SELECT 6"), (1, "SELECT 1")]
        self.assertEqual(query_fingerprint(first), query_fingerprint(list(reversed(first))))
        self.assertNotEqual(query_fingerprint(first), query_fingerprint([(6, "SELECT 7"), first[1]]))
        self.assertEqual(len(query_fingerprint(first)), 64)

    def test_error_and_timeout(self):
        def broken(_):
            raise RuntimeError("unsupported statement")
        self.assertEqual(benchmark_one(1, "SELECT 1", broken, 2, 0)["status"], "error")
        def timed_out(_):
            raise subprocess.TimeoutExpired("rusthouse", 1)
        self.assertEqual(benchmark_one(1, "SELECT 1", timed_out, 2, 0)["status"], "timeout")

    def test_coverage_distinguishes_execution_from_verified_results(self):
        statuses = ["passed", "unverified", "error", "mismatch", "missing_reference", "timeout"]
        results = [{"status": status} for status in statuses]
        self.assertEqual(
            summarize_coverage(results),
            {
                "total_queries": 6,
                "verified_queries": 1,
                "unverified_queries": 1,
                "failed_queries": 4,
                "execution_success_percent": 33.33,
                "verified_correct_percent": 16.67,
            },
        )
        self.assertEqual(summarize_coverage([])["execution_success_percent"], 0.0)

    def test_unverified_and_p90(self):
        result = benchmark_one(1, "SELECT 1", lambda _: b"ok\n", 2, 0)
        self.assertEqual(result["status"], "unverified")
        self.assertEqual(summary([1, 4, 2, 3, 5])["p90_seconds"], 5)

if __name__ == "__main__":
    unittest.main()

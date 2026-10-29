import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("analytics_runner", ROOT / "run.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class AnalyticsRunnerTests(unittest.TestCase):
    def test_pinned_queries_are_all_present(self):
        queries = runner.load_queries(ROOT / "queries.sql")
        self.assertEqual(len(queries), 43)
        self.assertEqual(queries[0], "SELECT COUNT(*) FROM hits")

    def test_preflight_checks_expected_output(self):
        calls = []

        def execute(sql, timeout):
            calls.append((sql, timeout))
            return b"42\n"

        runner.verify_input(execute, "SELECT COUNT(*) FROM sample", "42", 5)
        self.assertEqual(calls, [("SELECT COUNT(*) FROM sample", 5)])
        with self.assertRaisesRegex(ValueError, "preflight result differs"):
            runner.verify_input(execute, "SELECT COUNT(*) FROM sample", "43", 5)

    def test_preflight_does_not_ignore_value_whitespace(self):
        for actual in [b" 42\n", b"42 \n", b"\t42\n"]:
            with self.subTest(actual=actual):
                with self.assertRaisesRegex(ValueError, "preflight result differs"):
                    runner.verify_input(lambda _sql, _timeout: actual, "SELECT 1", "42", 5)

    def test_preflight_rejects_extra_empty_result_rows(self):
        for actual in (b"42\n\n", b"42\r\n\r\n", b"42\r"):
            with self.subTest(actual=actual):
                with self.assertRaisesRegex(ValueError, "preflight result differs"):
                    runner.verify_input(lambda _sql, _timeout: actual, "SELECT 1", "42", 5)

    def test_query_selection_preserves_order_and_checks_ranges(self):
        self.assertEqual(runner.select_indices("3,1-2,2", 43), [0, 1, 2])
        with self.assertRaises(ValueError):
            runner.select_indices("0", 43)
        with self.assertRaises(ValueError):
            runner.select_indices("44", 43)
        with self.assertRaises(ValueError):
            runner.select_indices("4-2", 43)

    def test_unknown_output_is_never_marked_verified(self):
        results = runner.run_benchmark(["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"1\n")
        self.assertEqual(results[0]["status"], "unverified")
        self.assertEqual(len(results[0]["seconds"]), 3)

    def test_latency_summary_includes_nearest_rank_p90(self):
        stats = runner.summarize_times([0.3, 0.1, 0.2])
        self.assertEqual(stats["min_seconds"], 0.1)
        self.assertEqual(stats["median_seconds"], 0.2)
        self.assertEqual(stats["p90_seconds"], 0.3)
        self.assertEqual(stats["max_seconds"], 0.3)
        self.assertIsNone(runner.summarize_times([]))

    def test_warmups_execute_without_affecting_timings(self):
        calls = []

        def execute(sql, _timeout):
            calls.append(sql)
            return b"1\\n"

        results = runner.run_benchmark(["SELECT 1"], [0], 3, 1, execute, warmups=2)
        self.assertEqual(len(calls), 5)
        self.assertEqual(len(results[0]["seconds"]), 3)
        self.assertEqual(results[0]["status"], "unverified")

    def test_reference_failure_reports_exact_mismatch_and_missing_files(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "q01.tsv"
            self.assertEqual(
                runner.reference_failure(path, b"1\n"),
                ("missing_reference", str(path)),
            )
            path.write_bytes(b"1\n")
            self.assertIsNone(runner.reference_failure(path, b"1\n"))
            self.assertEqual(
                runner.reference_failure(path, b"2\n"),
                ("mismatch", "query output differs from expected TSV"),
            )

    def test_reference_mismatch_is_a_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "q01.tsv").write_bytes(b"expected\n")
            results = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"actual\n", root
            )
            self.assertEqual(results[0]["status"], "mismatch")
            self.assertEqual(results[0]["seconds"], [])
            self.assertIsNone(results[0]["latency"])

    def test_reference_comparison_cost_does_not_inflate_elapsed_time(self):
        clock = [1_000_000_000]
        def execute(_sql, _timeout):
            clock[0] += 50_000_000
            return b"ok\n"
        def verify(_path, _output):
            clock[0] += 5_000_000_000
            return None
        with tempfile.TemporaryDirectory() as temp:
            (Path(temp) / "q01.tsv").write_bytes(b"ok\n")
            with mock.patch.object(runner.time, "perf_counter_ns", side_effect=lambda: clock[0]), \
                 mock.patch.object(runner, "reference_failure", side_effect=verify):
                item = runner.run_benchmark(
                    ["SELECT 1"], [0], 1, 1, execute, Path(temp)
                )[0]
        self.assertEqual(item["status"], "passed")
        self.assertEqual(item["seconds"], [0.05])

    def test_late_reference_failure_discards_partial_latency(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "q01.tsv").write_bytes(b"yes\n")
            calls = 0

            def changing(_sql, _timeout):
                nonlocal calls
                calls += 1
                return b"yes\n" if calls == 1 else b"no\n"

            result = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 10, changing, root, warmups=0
            )[0]
            self.assertEqual(result["status"], "mismatch")
            self.assertEqual(result["seconds"], [])
            self.assertIsNone(result["latency"])

    def test_missing_reference_is_a_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            results = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"1\n", Path(temp)
            )
            self.assertEqual(results[0]["status"], "missing_reference")

    def test_execution_error_is_recorded_without_hiding_other_queries(self):
        def execute(sql, _timeout):
            if sql == "bad":
                raise RuntimeError("unsupported")
            return b"good\n"

        results = runner.run_benchmark(["bad", "good"], [0, 1], 3, 1, execute)
        self.assertEqual([item["status"] for item in results], ["error", "unverified"])
        self.assertEqual(results[1]["seconds"].__len__(), 3)


    def test_missing_reference_does_not_execute_expensive_query(self):
        with tempfile.TemporaryDirectory() as temp:
            calls = []
            def execute(sql, timeout):
                calls.append((sql, timeout))
                return b"1\n"
            result = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 60, execute, Path(temp), 1
            )[0]
            self.assertEqual(result["status"], "missing_reference")
            self.assertEqual(result["seconds"], [])
            self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()

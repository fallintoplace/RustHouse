"""ClickBench runner correctness and coverage regression tests."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from run import (
    benchmark_one,
    coverage,
    load_projection,
    load_queries,
    main,
    matches_reference,
    preflight,
    run,
    select_queries,
)


class ClickBenchRunnerTests(unittest.TestCase):
    def test_loads_queries_with_original_numbers(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "queries.sql"
            path.write_text("-- official queries\nSELECT count(*) FROM hits;\n\n"
                            "# comment\nSELECT UserID FROM hits;\n")
            queries = load_queries(path)
            self.assertEqual(queries, [
                (1, "SELECT count(*) FROM hits"),
                (2, "SELECT UserID FROM hits"),
            ])
            self.assertEqual(select_queries(queries, "2"), [queries[1]])
            with self.assertRaisesRegex(ValueError, "unavailable"):
                select_queries(queries, "3")

    def test_projection_requires_explicit_column_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "projection.json"
            path.write_text(json.dumps({
                "available_columns": ["UserID"],
                "query_columns": {
                    "1": [],
                    "2": ["UserID"],
                    "3": ["SearchPhrase", "UserID"],
                },
            }))
            projection = load_projection(path)
            self.assertEqual(preflight(1, projection), (None, []))
            self.assertEqual(preflight(2, projection), (None, []))
            self.assertEqual(preflight(3, projection), (
                "skipped_missing_columns", ["SearchPhrase"]
            ))
            self.assertEqual(preflight(4, projection), ("unknown_requirements", []))
            self.assertEqual(preflight(4, None), (None, []))

    def test_bundled_projection_selects_exact_nine_smoke_queries(self):
        manifest = load_projection(Path(__file__).with_name("three-column-projection.json"))
        runnable = [number for number in range(1, 44)
                    if preflight(number, manifest)[0] is None]
        missing = [number for number in range(1, 44)
                   if preflight(number, manifest)[0] == "skipped_missing_columns"]
        unknown = [number for number in range(1, 44)
                   if preflight(number, manifest)[0] == "unknown_requirements"]
        self.assertEqual(runnable, [1, 2, 3, 4, 5, 8, 16, 20, 30])
        self.assertEqual(len(missing), 33)
        self.assertEqual(unknown, [24])

    def test_bad_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "projection.json"
            for document in (
                {"available_columns": ["UserID"], "query_columns": {"x": []}},
                {"available_columns": ["UserID"], "query_columns": {"1": "UserID"}},
                {"available_columns": "UserID", "query_columns": {}},
            ):
                path.write_text(json.dumps(document))
                with self.assertRaises(ValueError):
                    load_projection(path)

    def test_tsv_oracle_compares_large_integers_exactly(self):
        self.assertTrue(matches_reference(b"9007199254740993\n", b"9007199254740993\n"))
        self.assertFalse(matches_reference(b"9007199254740994\n", b"9007199254740993\n"))
        self.assertTrue(matches_reference(b"1.0000000000005\n", b"1\n"))
        self.assertFalse(matches_reference(b"1.000000000002\n", b"1\n"))
        self.assertFalse(matches_reference(b"0.0000000000001\n", b"0\n"))
        self.assertFalse(matches_reference(b"abc\n", b"def\n"))
        self.assertFalse(matches_reference(b"1\t2\n", b"1\n"))
        self.assertFalse(matches_reference(b"1\n2\n", b"1\n"))

    def test_reference_check_is_done_before_timing(self):
        correct = benchmark_one(1, "SELECT 1", lambda _: b"1\n", 3, 2, b"1\n")
        self.assertEqual(correct["status"], "passed")
        self.assertEqual(len(correct["timings_seconds"]), 3)
        wrong = benchmark_one(1, "SELECT 1", lambda _: b"2\n", 3, 2, b"1\n")
        self.assertEqual(wrong["status"], "mismatch")
        self.assertFalse(wrong["timings_seconds"])

    def test_validation_time_is_excluded_from_query_latency(self):
        clock = [10.0]
        def execute(_sql):
            clock[0] += 0.02
            return b"ok\n"
        def validate(_output, _reference):
            clock[0] += 4.0
            return True
        with mock.patch("run.time.perf_counter", side_effect=lambda: clock[0]), \
             mock.patch("run.matches_reference", side_effect=validate):
            result = benchmark_one(1, "SELECT 1", execute, 1, 0, b"ok\n")
        self.assertEqual(result["status"], "passed")
        self.assertAlmostEqual(result["timings_seconds"][0], 0.02, places=6)

    def test_late_reference_mismatch_discards_all_timings(self):
        calls = 0

        def changing(_sql):
            nonlocal calls
            calls += 1
            return b"ok\n" if calls == 1 else b"wrong\n"

        result = benchmark_one(1, "SELECT 1", changing, 3, 0, b"ok\n")
        self.assertEqual(result["status"], "mismatch")
        self.assertEqual(result["timings_seconds"], [])
        self.assertIsNone(result["latency"])

    def test_errors_are_separate_from_missing_columns(self):
        def parse_error(_):
            raise RuntimeError("SQL parse error: unexpected JOIN")
        self.assertEqual(
            benchmark_one(1, "SELECT bad", parse_error, 2, 0)["status"],
            "parse_error",
        )
        def timeout(_):
            raise subprocess.TimeoutExpired("rusthouse", 2)
        self.assertEqual(
            benchmark_one(1, "SELECT bad", timeout, 2, 0)["status"],
            "timeout",
        )

    def test_skips_omitted_columns_without_executing_queries(self):
        calls = []
        def execute(sql):
            calls.append(sql)
            return b"1\n"

        projection = {
            "available": {"UserID"},
            "required": {"1": [], "2": ["UserID"], "3": ["SearchPhrase"]},
        }
        queries = [(1, "q1"), (2, "q2"), (3, "q3"), (4, "q4")]
        results = run(queries, execute, 1, 0, projection)
        self.assertEqual(calls, ["q1", "q2"])
        self.assertEqual([result["status"] for result in results], [
            "unverified", "unverified", "skipped_missing_columns", "unknown_requirements",
        ])
        self.assertEqual(coverage(results), {
            "total_queries": 4,
            "eligible_queries": 2,
            "executed_queries": 2,
            "verified_queries": 0,
            "skipped_missing_columns": 1,
            "unknown_requirements": 1,
            "failed_eligible_queries": 0,
            "execution_success_percent_of_eligible": 100.0,
            "verified_percent_of_total": 0.0,
        })

    def test_missing_reference_does_not_count_as_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            results = run([(1, "SELECT 1")], lambda _: b"1\n", 1, 0,
                          references=Path(directory))
        self.assertEqual(results[0]["status"], "missing_reference")
        self.assertEqual(coverage(results)["verified_queries"], 0)

    def test_cli_report_preserves_selected_query_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "queries.sql").write_text("SELECT 1;\nSELECT 2;\n")
            output = root / "nested" / "report.json"
            with mock.patch("run.cli_sql", return_value=b"2\n") as invoked:
                result = main([
                    "--binary", str(root / "rusthouse"),
                    "--database", str(root / "db"),
                    "--queries-file", str(root / "queries.sql"),
                    "--only", "2", "--runs", "1", "--warmups", "0",
                    "--output", str(output),
                ])
            self.assertEqual(result, 0)
            self.assertEqual(invoked.call_count, 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["query_file_count"], 2)
            self.assertFalse(report["complete_official_query_list"])
            self.assertEqual(report["results"][0]["query"], 2)
            self.assertEqual(report["coverage"]["executed_queries"], 1)


if __name__ == "__main__":
    unittest.main()

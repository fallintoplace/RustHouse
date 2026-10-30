"""Smoke tests for generated query references and latency reporting."""

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("latency_runner", ROOT / "run.py")
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


class GeneratedQueryTests(unittest.TestCase):
    def test_has_twenty_unique_parameterized_cases_with_exact_references(self):
        cases = runner.make_cases(512)
        self.assertEqual(len(cases), 20)
        self.assertEqual(len({case.name for case in cases}), 20)
        self.assertEqual(len({case.sql for case in cases}), 20)
        self.assertTrue(all("numbers(512)" in case.sql or "generate_series(0, 511, 3)" in case.sql
                            for case in cases))
        by_name = {case.name: case for case in cases}
        self.assertEqual(by_name["count-mod-7"].expected, b"74\n")
        self.assertEqual(by_name["sum-half"].expected, b"98176\n")
        self.assertEqual(by_name["count-or"].expected, b"146\n")
        self.assertEqual(by_name["group-short"].expected, b"".join(
            f"{number}\t1\n".encode() for number in range(19, 9, -1)
        ))
        self.assertEqual(by_name["filtered-offset"].expected, b"".join(
            f"{7 * n}\n".encode() for n in range(50, 70)
        ))
        self.assertEqual(by_name["series-top-desc"].expected.splitlines()[0], b"504")
        self.assertEqual(by_name["sum-mod-10"].sql,
                         "SELECT sum(number) FROM numbers(512) WHERE number % 10 = 0")
        self.assertEqual(by_name["sum-mod-10"].expected, b"13260\n")

    def test_large_scale_stays_bounded_and_oracles_remain_exact(self):
        cases = {case.name: case for case in runner.make_cases(1_000_000)}
        self.assertEqual(cases["count-all"].expected, b"1000000\n")
        self.assertEqual(cases["count-distinct"].expected, b"1000000\n")
        self.assertEqual(len(cases["filtered-top-desc"].expected.splitlines()), 20)
        with self.assertRaises(ValueError):
            runner.make_cases(511)
        with self.assertRaises(ValueError):
            runner.make_cases(1_000_001)

    def test_query_selection_has_stable_ids_and_rejects_invalid_ranges(self):
        cases = runner.make_cases(512)
        self.assertEqual([i for i, _ in runner.select_cases(cases, "3,1-2,2,20")], [1, 2, 3, 20])
        for invalid in ("", "0", "21", "5-3", "3-21", "foo", "1,,3"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                runner.select_cases(cases, invalid)

    def test_warmups_are_untimed_and_every_result_is_verified(self):
        cases = runner.select_cases(runner.make_cases(512), "1,2")
        output = {case.sql: case.expected for _, case in cases}
        calls = []

        def execute(sql):
            calls.append(sql)
            return output[sql]

        results = runner.run_benchmark(cases, execute, runs=3, warmups=2)
        self.assertEqual(len(calls), 10)
        self.assertEqual([record["status"] for record in results], ["passed", "passed"])
        self.assertTrue(all(len(record["seconds"]) == 3 for record in results))
        self.assertTrue(all(record["latency"]["p90_seconds"] >= record["latency"]["median_seconds"]
                            for record in results))

    def test_mismatch_disallows_partial_timings_and_other_cases_continue(self):
        cases = runner.select_cases(runner.make_cases(512), "1-2")
        first_sql = cases[0][1].sql

        def execute(sql):
            return b"wrong\n" if sql == first_sql else cases[1][1].expected

        results = runner.run_benchmark(cases, execute, runs=2, warmups=1)
        self.assertEqual([record["status"] for record in results], ["mismatch", "passed"])
        self.assertEqual(results[0]["seconds"], [])
        self.assertIsNone(results[0]["latency"])
        self.assertEqual(len(results[1]["seconds"]), 2)

    def test_timeout_and_error_are_recorded_as_failures(self):
        case = runner.make_cases(512)[0]
        for failure, status in ((subprocess.TimeoutExpired("binary", 1), "timeout"),
                                (RuntimeError("bad SQL"), "error")):
            with self.subTest(status=status):
                def execute(_sql):
                    raise failure
                result = runner.benchmark_one(1, case, execute, 2, 0)
                self.assertEqual(result["status"], status)
                self.assertIsNone(result["latency"])

    def test_main_rejects_nonfinite_timeout(self):
        for timeout in ("nan", "inf"):
            with self.subTest(timeout=timeout):
                with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as failure:
                    runner.main(["--binary", "dummy", "--timeout", timeout])
                self.assertEqual(failure.exception.code, 2)

    def test_main_reports_machine_context_and_writes_identical_json(self):
        reference = runner.make_cases(512)[0].expected
        with tempfile.TemporaryDirectory() as folder, \
                mock.patch.object(runner, "invoke", return_value=reference), \
                mock.patch.object(runner, "optional_version", return_value="test"):
            path = Path(folder) / "nested" / "results.json"
            output, details = io.StringIO(), io.StringIO()
            with redirect_stdout(output), redirect_stderr(details):
                code = runner.main([
                    "--binary", "fake-rusthouse", "--rows", "512", "--runs", "2",
                    "--warmups", "1", "--only", "1", "--output", str(path),
                ])
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(output.getvalue()), json.loads(path.read_text()))
            self.assertEqual(json.loads(path.read_text())["machine"]["rustc"], "test")
            self.assertIn("Median ms", details.getvalue())


    def test_sql_benchmark_capture_rejects_oversized_results(self):
        def fake_run(command, **kwargs):
            kwargs["stdout"].write(b"x" * (256 * 1024 + 1))
            return subprocess.CompletedProcess(command, 0, b"", b"")
        with mock.patch.object(runner.subprocess, "run", side_effect=fake_run) as execute:
            with self.assertRaisesRegex(ValueError, "256 KiB"):
                runner.invoke("rusthouse", "SELECT 1", 3)
        self.assertEqual(execute.call_args.kwargs["stderr"], subprocess.PIPE)

if __name__ == "__main__":
    unittest.main()

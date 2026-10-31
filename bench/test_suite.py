"""Regression tests for the shared development and benchmark entry point."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench import __main__ as runner


class BenchmarkEntryPointTests(unittest.TestCase):
    def test_all_rust_suites_have_correct_release_commands(self):
        commands = {suite: runner.benchmark_command(suite, 65536, 7, 413)
                    for suite in runner.BENCHMARK_SUITES}
        for command in commands.values():
            self.assertEqual(command[:5], ["cargo", "bench", "-p", "rusthouse-query", "--bench"])
            self.assertEqual(command[6:11], ["--locked", "--", "--rows", "65536", "--samples"])
            self.assertEqual(command[11], "7")
        self.assertEqual(commands["parts-sorted"][-3:], ["--layout", "sorted", "--csv"])
        self.assertEqual(commands["parts-shuffled"][-3:], ["--layout", "shuffled", "--csv"])
        self.assertEqual(commands["grouped"][-2:], ["--stations", "413"])

    def test_invalid_scale_is_rejected_before_spawning_cargo(self):
        for options in (
            ["--rows", "0"],
            ["--rows", "1000001"],
            ["--samples", "0"],
            ["--samples", "2"],
            ["--stations", "0"],
            ["--rows", "20", "--stations", "21"],
        ):
            with self.subTest(options=options), mock.patch.object(runner, "benchmark") as execute:
                with self.assertRaises(SystemExit) as caught:
                    runner.main(["benchmark", *options])
                self.assertEqual(caught.exception.code, 2)
                execute.assert_not_called()

    def test_report_preserves_success_and_failure_with_output(self):
        with tempfile.TemporaryDirectory() as directory:
            responses = [
                subprocess.CompletedProcess([], 0, "workload,median_ms\ncount,1.2\n", ""),
                subprocess.CompletedProcess([], 17, "partial output\n", "failed query\n"),
            ]
            with mock.patch.object(runner, "optional_version", return_value="pinned"), \
                 mock.patch.object(runner.subprocess, "run", side_effect=responses) as process:
                result = runner.benchmark(
                    ("parts-sorted", "grouped"), 65536, 7, 413, Path(directory)
                )
            self.assertEqual(result, 1)
            self.assertEqual(process.call_count, 2)
            report = json.loads((Path(directory) / "manifest.json").read_text())
            self.assertEqual(report["rows"], 65536)
            self.assertEqual([record["status"] for record in report["results"]], ["passed", "failed"])
            self.assertEqual(report["results"][1]["exit_code"], 17)
            self.assertIn("count,1.2", (Path(directory) / "parts-sorted.csv").read_text())
            self.assertEqual((Path(directory) / "grouped.stderr.txt").read_text(), "failed query\n")

    def test_suite_timeout_records_a_failure_in_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.object(runner, "optional_version", return_value=None), \
                 mock.patch.object(runner.subprocess, "run",
                                   side_effect=subprocess.TimeoutExpired(["cargo"], 0.01)) as run:
                failed = runner.benchmark(
                    ("queries",), 65536, 7, 413, directory, suite_timeout=0.01
                )
            self.assertEqual(failed, 1)
            self.assertEqual(run.call_args.kwargs["timeout"], 0.01)
            record = json.loads((Path(directory) / "manifest.json").read_text())["results"][0]
            self.assertEqual(record["status"], "failed")
            self.assertEqual(record["exit_code"], 124)
            self.assertIn("timed out", (Path(directory) / "queries.stderr.txt").read_text())

    def test_invalid_suite_timeouts_fail_before_launch(self):
        for value in ("0", "-1", "nan", "inf"):
            with self.subTest(value=value), mock.patch.object(runner, "benchmark") as execute:
                with self.assertRaises(SystemExit) as raised:
                    runner.main(["benchmark", "--suite-timeout", value])
                self.assertEqual(raised.exception.code, 2)
                execute.assert_not_called()

    def test_missing_cargo_still_writes_failure_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.object(runner, "optional_version", return_value=None), \
                 mock.patch.object(runner.subprocess, "run", side_effect=FileNotFoundError("cargo")):
                result = runner.benchmark(("queries",), 65536, 7, 413, directory)
            self.assertEqual(result, 1)
            manifest = json.loads((Path(directory) / "manifest.json").read_text())
            self.assertEqual(manifest["results"][0]["status"], "failed")
            self.assertIn("cargo", (Path(directory) / "queries.stderr.txt").read_text())

    def test_workload_forwards_arguments_without_reinterpreting_them(self):
        with mock.patch.object(runner, "run_command", return_value=0) as invoke:
            result = runner.main(["workload", "joins", "--", "--binary", "target/release/rusthouse", "--runs", "3"])
        self.assertEqual(result, 0)
        self.assertEqual(invoke.call_args.args[0], "joins")
        self.assertEqual(
            invoke.call_args.args[1],
            [sys.executable, str(runner.ROOT / "bench/joins/run.py"),
             "--binary", "target/release/rusthouse", "--runs", "3"],
        )

    def test_physical_scan_workload_forwards_to_existing_scale_runner(self):
        with mock.patch.object(runner, "run_command", return_value=0) as invoke:
            result = runner.main([
                "workload", "physical-scan", "--",
                "--binary", "target/debug/rusthouse", "--rows", "2048",
            ])
        self.assertEqual(result, 0)
        self.assertEqual(
            invoke.call_args.args[1],
            [sys.executable, str(runner.ROOT / "bench/scale/physical_scan.py"),
             "--binary", "target/debug/rusthouse", "--rows", "2048"],
        )

    def test_analytics_suite_uses_package_import_path(self):
        with mock.patch.object(runner, "run_command", return_value=0) as invoke:
            result = runner.main([
                "workload", "analytics-suite", "--",
                "--binary", "target/debug/rusthouse",
            ])
        self.assertEqual(result, 0)
        self.assertEqual(
            invoke.call_args.args[1],
            [sys.executable, "-m", "bench.analytics.suite_runner",
             "--binary", "target/debug/rusthouse"],
        )

    def test_python_tests_keep_running_after_a_failure(self):
        with mock.patch.object(
            runner, "run_command", side_effect=[0, 1] + [0] * (len(runner.PYTHON_SUITES) - 2)
        ) as invoke:
            self.assertEqual(runner.python_tests(), 1)
            self.assertEqual(invoke.call_count, len(runner.PYTHON_SUITES))
            self.assertTrue(all(call.args[1][0] == sys.executable for call in invoke.call_args_list))


if __name__ == "__main__":
    unittest.main()


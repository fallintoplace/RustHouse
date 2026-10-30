"""Tests for shared workload execution and latency reporting."""

import io
import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

from bench.common import cli_sql, emit_json, latency_summary


class SharedWorkloadTests(unittest.TestCase):
    def test_nearest_rank_percentiles_and_no_samples(self):
        self.assertIsNone(latency_summary([]))
        self.assertEqual(latency_summary([0.3, 0.1, 0.2]), {
            "min_seconds": 0.1, "median_seconds": 0.2,
            "p90_seconds": 0.3, "max_seconds": 0.3,
        })
        self.assertEqual(latency_summary([1, 2, 3, 4, 5])["p90_seconds"], 5)
        with self.assertRaises(ValueError):
            latency_summary([float("nan")])
        with self.assertRaises(ValueError):
            latency_summary([-1])

    def test_cli_preserves_raw_bytes_for_exact_comparisons(self):
        def execute_sql(command, **kwargs):
            kwargs["stdout"].write(b"1\n")
            return subprocess.CompletedProcess(command, 0, None, b"")
        with mock.patch("bench.common.subprocess.run", side_effect=execute_sql) as execute:
            result = cli_sql("rusthouse", "db", "SELECT 1", 5)
        self.assertEqual(result, b"1\n")
        self.assertEqual(
            execute.call_args.args[0],
            ["rusthouse", "sql", "SELECT 1", "--database", "db", "--format", "tsv-no-header"],
        )
        self.assertNotEqual(execute.call_args.kwargs["stderr"], subprocess.PIPE)
        self.assertEqual(execute.call_args.kwargs["timeout"], 5)
        self.assertNotEqual(execute.call_args.kwargs["stdout"], subprocess.PIPE)

    def test_cli_rejects_result_larger_than_64_mib(self):
        from bench.common import MAX_SQL_STDOUT_BYTES
        def write_oversized(command, **kwargs):
            kwargs["stdout"].truncate(MAX_SQL_STDOUT_BYTES + 1)
            return subprocess.CompletedProcess(command, 0, None, b"")
        with mock.patch("bench.common.subprocess.run", side_effect=write_oversized):
            with self.assertRaisesRegex(ValueError, "64 MiB"):
                cli_sql("rusthouse", "db", "SELECT 1", 5)

    def test_cli_caps_large_stderr_in_memory(self):
        def noisy_sql(command, **kwargs):
            kwargs["stderr"].write(b"error " + b"x" * 100_000)
            return subprocess.CompletedProcess(command, 1)
        with mock.patch("bench.common.subprocess.run", side_effect=noisy_sql):
            with self.assertRaises(RuntimeError) as raised:
                cli_sql("rusthouse", "db", "bad", 1)
        self.assertLess(len(str(raised.exception)), 2100)

    def test_cli_json_oracle_mode_is_explicit_and_read_only(self):
        with mock.patch(
            "bench.common.subprocess.run",
            return_value=subprocess.CompletedProcess([], 0, b'{"x":"1"}\\n', b""),
        ) as run:
            cli_sql("rusthouse", "db", "SELECT 1", 5, select_format="jsonl")
        self.assertEqual(run.call_args.args[0][-2:], ["--format", "jsonl"])
        with self.assertRaisesRegex(ValueError, "only with SELECT"):
            cli_sql("rusthouse", "db", "DROP TABLE t", 5, select_format="jsonl")
        with self.assertRaisesRegex(ValueError, "unsupported"):
            cli_sql("rusthouse", "db", "SELECT 1", 5, select_format="unsafe")

    def test_cli_mutations_omit_select_only_format(self):
        for statement in (
            "CREATE TABLE events (id UInt64)",
            "COPY events FROM 'events.tsv' WITH (FORMAT TSV)",
            "INSERT INTO events VALUES (1)",
        ):
            with self.subTest(statement=statement):
                with mock.patch(
                    "bench.common.subprocess.run",
                    return_value=subprocess.CompletedProcess([], 0, b"", b""),
                ) as execute:
                    self.assertEqual(cli_sql("rusthouse", "db", statement, 5), b"")
                self.assertEqual(
                    execute.call_args.args[0],
                    ["rusthouse", "sql", statement, "--database", "db"],
                )

    def test_cli_error_and_timeout_are_not_swallowed(self):
        def fail_sql(command, **kwargs):
            kwargs["stderr"].write(b"failure")
            return subprocess.CompletedProcess(command, 2)
        with mock.patch("bench.common.subprocess.run", side_effect=fail_sql):
            with self.assertRaisesRegex(RuntimeError, "exit 2: failure"):
                cli_sql("rusthouse", "db", "bad", 1)
        with mock.patch("bench.common.subprocess.run", side_effect=subprocess.TimeoutExpired("rusthouse", 1)):
            with self.assertRaises(subprocess.TimeoutExpired):
                cli_sql("rusthouse", "db", "slow", 1)

    def test_json_report_rejects_nonfinite_measurements(self):
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(value=value), self.assertRaises(ValueError):
                emit_json({"latency_seconds": value})

    def test_json_output_matches_saved_file(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "nested" / "reports" / "report.json"
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                emit_json({"verified": True}, target)
            self.assertEqual(target.read_text(), buffer.getvalue())
            self.assertEqual(json.loads(buffer.getvalue()), {"verified": True})


    def test_failed_report_publish_preserves_existing_json(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "result.json"
            destination.write_text('{"previous": true}\n')
            with mock.patch("bench.common.os.replace", side_effect=OSError("rename failed")):
                with self.assertRaisesRegex(OSError, "rename failed"):
                    emit_json({"next": True}, destination)
            self.assertEqual(destination.read_text(), '{"previous": true}\n')
            self.assertEqual(list(Path(directory).glob(".result.json.*.tmp")), [])

    def test_latency_summary_rejects_wrong_types_and_huge_integers(self):
        for value in ("1", True, None, 10 ** 400, float("inf")):
            with self.subTest(value=repr(value)):
                with self.assertRaisesRegex(ValueError, "timing samples"):
                    latency_summary([value])

if __name__ == "__main__":
    unittest.main()


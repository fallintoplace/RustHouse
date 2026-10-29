import tempfile
import unittest
from pathlib import Path

from bench.tpch.paired_duckdb import ABBA, BAAB, benchmark_pair, command, compare, load_paired_queries


class PairedDuckDbTests(unittest.TestCase):
    def test_strict_full_sql_rejects_simplified_queries_duckdb(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            for number in range(1, 23):
                (path / f"q{number:02d}.sql").write_text(f"SELECT {number};")
            self.assertEqual(len(load_paired_queries(path)), 22)
            with self.assertRaises(ValueError):
                load_paired_queries(path, require_canonical=True)

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
        self.assertEqual(result["rusthouse_p90_seconds"], 0.002)
        self.assertEqual(result["duckdb_p90_seconds"], 0.001)
        self.assertEqual(result["duckdb_p90_over_rusthouse"], 0.5)

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

    def test_cli_output_is_spooled_and_capped(self):
        import subprocess
        from unittest import mock
        from bench.tpch import paired_duckdb as runner

        def fake_run(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"12345")
            return subprocess.CompletedProcess(command, 0, b"", b"")

        with mock.patch.object(runner.subprocess, "run", side_effect=fake_run), \
             mock.patch.object(runner, "MAX_OUTPUT_BYTES", 4):
            with self.assertRaisesRegex(ValueError, "16 MiB"):
                runner.invoke(["fake"], 5)

    def test_oversized_reference_is_not_executed(self):
        from unittest import mock
        from bench.tpch import paired_duckdb as runner

        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "q01.tsv").write_bytes(b"12345")
            with mock.patch.object(runner, "MAX_OUTPUT_BYTES", 4), \
                 mock.patch.object(runner, "invoke", side_effect=AssertionError("executed")):
                report = compare("unused", "db", "duckdb", "ddb",
                                 [(1, "SELECT 1")], Path(directory), 1, 0, 10)
            self.assertFalse(report["all_queries_verified"])
            self.assertIn("exceeds 16 MiB", report["results"][0]["error"])

    def test_missing_reference_never_claims_speedup(self):
        with tempfile.TemporaryDirectory() as root:
            report = compare("unused", "db", "duckdb", "ddb",
                             [(1, "SELECT 1")], Path(root), 1, 0, 10)
        self.assertFalse(report["all_queries_verified"])
        self.assertIsNone(report["speedup_geomean_duckdb_over_rusthouse"])
        self.assertEqual(report["results"][0]["status"], "error")



class PairedDuckDBReferenceSymlinkTests(unittest.TestCase):
    def test_symlink_reference_is_rejected_without_running_queries(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.tsv"
            source.write_bytes(b"1\\n")
            link = root / "q01.tsv"
            try:
                link.symlink_to(source)
            except (OSError, NotImplementedError):
                self.skipTest("symlinks unavailable")
            with mock.patch("bench.tpch.paired_duckdb.invoke", side_effect=AssertionError("executed")):
                result = compare("rusthouse", "db", "duckdb", "db",
                                 [(1, "SELECT 1")], root, 1, 0, 10)
            self.assertFalse(result["all_queries_verified"])
            self.assertIn("unsafe", result["results"][0]["error"])

if __name__ == "__main__":
    unittest.main()

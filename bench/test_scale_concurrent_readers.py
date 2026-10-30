import unittest
from types import SimpleNamespace

from bench.scale.concurrent_readers import run_readers


class ConcurrentReadersTests(unittest.TestCase):
    def test_reference_file_read_is_bounded_and_no_symlink(self):
        import tempfile
        from pathlib import Path
        from bench.scale.concurrent_readers import read_reference
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "expected.tsv"
            path.write_bytes(b"1\n")
            self.assertEqual(read_reference(path), b"1\n")
            (Path(root) / "alias.tsv").symlink_to(path)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                read_reference(Path(root) / "alias.tsv")
            path.write_bytes(b"x" * (1024 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "exceeds 1 MiB"):
                read_reference(path)


    def test_exact_reference_checked_for_every_reader(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"2\n", stderr=b"")
        report = run_readers("rusthouse", "part", "SELECT count(*) FROM part",
                             b"2\n", clients=3, rounds=2, execute=run)
        self.assertEqual(report["planned"], 6)
        self.assertEqual(report["verified"], 6)
        self.assertTrue(report["all_verified"])
        self.assertGreater(report["verified_queries_per_second"], 0)

    def test_mismatch_is_not_success(self):
        result = run_readers("rusthouse", "part", "SELECT 1", b"1\n",
                             clients=2, rounds=1, execute=lambda *a, **k:
                             SimpleNamespace(returncode=0, stdout=b"2\n", stderr=b""))
        self.assertEqual(result["verified"], 0)
        self.assertFalse(result["all_verified"])

    def test_rejects_float_worker_counts_and_nonfinite_timeout(self):
        for kwargs in ({"clients": 1.0}, {"rounds": 2.5},
                       {"timeout": float("nan")}, {"timeout": float("inf")}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    run_readers("binary", "part", "SELECT 1", b"1\n", **kwargs)

    def test_caps_total_jobs_and_output_bytes(self):
        with self.assertRaisesRegex(ValueError, "256"):
            run_readers("binary", "part", "SELECT 1", b"", clients=64, rounds=5)
        with self.assertRaisesRegex(ValueError, "1 MiB"):
            run_readers("binary", "part", "SELECT 1", b"x" * (1024 * 1024 + 1))

    def test_allows_semicolons_within_sql_string_literals(self):
        from bench.scale.concurrent_readers import _has_separator
        for statement in ("SELECT ';' AS x", 'SELECT ";" AS x',
                          "SELECT 'it''s;safe' AS x", "SELECT 'a\\';b'"):
            self.assertFalse(_has_separator(statement), statement)
        for statement in ("SELECT 1; DROP TABLE t", "SELECT 'unterminated",
                          'SELECT "unfinished'):
            self.assertTrue(_has_separator(statement), statement)
        report = run_readers("rusthouse", "part", "SELECT ';';", b";\n",
                             clients=1, rounds=1, execute=lambda *_a, **_k:
                             SimpleNamespace(returncode=0, stdout=b";\n", stderr=b""))
        self.assertTrue(report["all_verified"])

    def test_rejects_mutations_and_multistatement_sql(self):
        for sql in ("DELETE FROM part", "SELECT 1; DROP TABLE events"):
            with self.assertRaisesRegex(ValueError, "read-only"):
                run_readers("rusthouse", "part", sql, b"1\n")


    def test_reports_nearest_rank_tail_latency_only_for_verified_reads(self):
        def execute(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"1\n", stderr=b"")
        report = run_readers("binary", "part", "SELECT 1", b"1\n",
                             clients=2, rounds=2, execute=execute)
        self.assertEqual(report["verified"], 4)
        self.assertGreaterEqual(report["p95_query_seconds"], report["median_query_seconds"])
        self.assertGreaterEqual(report["p99_query_seconds"], report["p95_query_seconds"])
        failed = run_readers("binary", "part", "SELECT 1", b"2\n",
                             clients=1, rounds=1, execute=execute)
        self.assertIsNone(failed["p95_query_seconds"])
        self.assertIsNone(failed["p99_query_seconds"])

if __name__ == "__main__":
    unittest.main()

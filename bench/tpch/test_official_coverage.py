import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from official_coverage import coverage, clean_sql


class CoverageTests(unittest.TestCase):
    def test_missing_queries_remain_explicit_and_unverified_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as root:
            queries = Path(root)
            (queries / "q06.sql").write_text("-- ignored\nSELECT 6;\n")
            report = coverage("fake", "db", queries, runs=1, warmups=0,
                              execute=lambda sql: b"6\n")
            self.assertEqual(report["coverage"]["total_queries"], 22)
            self.assertEqual(report["coverage"]["unverified_queries"], 1)
            self.assertEqual(report["coverage"]["failed_queries"], 21)
            self.assertEqual(report["results"][5]["status"], "unverified")
            from run import query_fingerprint
            self.assertEqual(report["query_sha256"],
                             query_fingerprint([(6, "SELECT 6")]))
            self.assertEqual(clean_sql(queries / "q06.sql"), "SELECT 6")
            refs = queries / "refs"
            refs.mkdir()
            checked = coverage("fake", "db", queries, refs, runs=1, warmups=0,
                               execute=lambda sql: b"6\n")
            self.assertEqual(checked["results"][5]["status"], "missing_reference")

    def test_parallel_inventory_preserves_canonical_query_order(self):
        with tempfile.TemporaryDirectory() as root:
            queries = Path(root)
            for number in (1, 2, 3, 6):
                (queries / f"q{number:02d}.sql").write_text(
                    f"SELECT {number};", encoding="utf-8"
                )
            report = coverage("fake", "db", queries, runs=1, warmups=0,
                              jobs=3, execute=lambda sql: b"ok\\n")
            self.assertEqual([item["query"] for item in report["results"]],
                             list(range(1, 23)))
            self.assertEqual(report["concurrent_queries"], 3)
            self.assertEqual(report["coverage"]["unverified_queries"], 4)
            self.assertEqual(report["coverage"]["failed_queries"], 18)

    def test_rejects_invalid_parallel_query_counts(self):
        for jobs in (0, 23, True, 1.5):
            with self.subTest(jobs=jobs):
                with self.assertRaisesRegex(ValueError, "timing"):
                    coverage("fake", "db", "/unused", jobs=jobs)

    def test_rejects_nonfinite_timeouts_and_noninteger_run_counts(self):
        for options in ({"timeout": float("nan")}, {"timeout": float("inf")},
                        {"runs": True}, {"warmups": 1.5}):
            with self.subTest(options=options):
                with self.assertRaisesRegex(ValueError, "timing"):
                    coverage("fake", "db", "/unused", execute=lambda _: b"", **options)

    def test_query_inventory_matches_cli_64k_statement_limit(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "q01.sql").write_text("SELECT '" + "x" * 5000 + "'")
            (path / "q02.sql").write_text("SELECT '" + "x" * 65536 + "'")
            observed = []
            def execute(sql):
                observed.append(sql)
                return b"1\n"
            report = coverage("fake", "db", path, runs=1, warmups=0,
                              execute=execute)
            self.assertEqual(report["results"][0]["status"], "unverified")
            self.assertEqual(report["results"][1]["status"], "unsupported_query")
            self.assertEqual(len(observed), 1)

    def test_oversized_references_fail_before_running_sql(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "q01.sql").write_text("SELECT 1;")
            refs = path / "refs"
            refs.mkdir()
            (refs / "q01.tsv").write_bytes(b"123456\n")
            called = []
            with patch("official_coverage.MAX_REFERENCE_BYTES", 3):
                report = coverage("fake", "db", path, refs, runs=1, warmups=0,
                                  execute=lambda sql: called.append(sql))
            self.assertEqual(report["results"][0]["status"], "reference_error")
            self.assertEqual(called, [])

    def test_reference_match_and_mismatch_are_distinct(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "q01.sql").write_text("SELECT 1;")
            refs = path / "refs"
            refs.mkdir()
            (refs / "q01.tsv").write_bytes(b"1\n")
            good = coverage("fake", "db", path, refs, runs=1, warmups=0,
                            execute=lambda _: b"1\n")
            bad = coverage("fake", "db", path, refs, runs=1, warmups=0,
                           execute=lambda _: b"2\n")
            self.assertEqual(good["results"][0]["status"], "passed")
            self.assertEqual(bad["results"][0]["status"], "mismatch")


class BoundedOfficialSQLInventoryTests(unittest.TestCase):
    def test_large_raw_sql_is_reported_without_query_execution(self):
        with tempfile.TemporaryDirectory() as root:
            queries = Path(root)
            (queries / "q01.sql").write_bytes(b"-- verbose\n" * 20 + b"SELECT 1\n")
            invoked = []
            with patch("official_coverage.MAX_QUERY_FILE_BYTES", 32):
                report = coverage("unused", "db", queries, runs=1, warmups=0,
                                  execute=lambda sql: invoked.append(sql))
            self.assertEqual(report["results"][0]["status"], "unsupported_query")
            self.assertIn("64 KiB", report["results"][0]["error"])
            self.assertEqual(invoked, [])

if __name__ == "__main__":
    unittest.main()

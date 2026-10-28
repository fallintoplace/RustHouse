import tempfile
import unittest
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
            self.assertEqual(clean_sql(queries / "q06.sql"), "SELECT 6")
            refs = queries / "refs"
            refs.mkdir()
            checked = coverage("fake", "db", queries, refs, runs=1, warmups=0,
                               execute=lambda sql: b"6\n")
            self.assertEqual(checked["results"][5]["status"], "missing_reference")

    def test_query_inventory_matches_cli_64k_statement_limit(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)
            (path / "q01.sql").write_text("SELECT 1" + " " * 5000)
            (path / "q02.sql").write_text("SELECT 2" + " " * 65536)
            observed = []
            def execute(sql):
                observed.append(sql)
                return b"1\\n"
            report = coverage("fake", "db", path, runs=1, warmups=0,
                              execute=execute)
            self.assertEqual(report["results"][0]["status"], "unverified")
            self.assertEqual(report["results"][1]["status"], "unsupported_query")
            self.assertEqual(len(observed), 1)

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


if __name__ == "__main__":
    unittest.main()

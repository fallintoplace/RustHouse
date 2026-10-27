import tempfile
import unittest
from pathlib import Path

from make_fixture import generate
from smoke import run_smoke


class SmokeTests(unittest.TestCase):
    def test_full_lifecycle_and_reference_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            fixture = generate(source, 12, 7)
            expected = (source / "references" / "q06.tsv").read_bytes()
        statements = []
        def execute(sql):
            statements.append(sql)
            if sql.startswith("SELECT count(*)"):
                return b"12\n"
            if sql.startswith("SELECT SUM("):
                return expected
            return b""
        report = run_smoke(Path("/fake/rusthouse"), rows=12, seed=7, runs=2,
                           warmups=1, execute=execute)
        self.assertEqual(report["coverage"]["verified_queries"], 1)
        self.assertEqual(report["results"][0]["status"], "passed")
        self.assertEqual(report["expected_matches"], fixture["matched_rows"])
        self.assertEqual(len(statements), 6)
        self.assertFalse(report["official_tpch"])

    def test_rejects_corrupt_import_and_bad_arguments(self):
        with self.assertRaisesRegex(ValueError, "positive"):
            run_smoke(Path("fake"), rows=0)
        with self.assertRaisesRegex(ValueError, "row count"):
            run_smoke(Path("fake"), rows=2, execute=lambda sql: b"1\n")

    def test_wrong_results_are_never_reported_as_verified(self):
        def execute(sql):
            if sql.startswith("SELECT count(*)"):
                return b"2\n"
            return b"wrong\n" if sql.startswith("SELECT SUM(") else b""
        result = run_smoke(Path("fake"), rows=2, runs=2, execute=execute)
        self.assertEqual(result["results"][0]["status"], "mismatch")
        self.assertEqual(result["coverage"]["verified_queries"], 0)


if __name__ == "__main__":
    unittest.main()

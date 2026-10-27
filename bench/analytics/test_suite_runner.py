"""Oracle, fixture and execution contract tests for synthetic analytics."""
import json
import tempfile
import unittest
from pathlib import Path

from bench.analytics.suite_runner import (
    benchmark_one, fact_row, load_workload, make_fixture, run_suite, sqlite_reference,
)

WORKLOADS = Path(__file__).resolve().parent / "workloads"


class AnalyticsSuiteTests(unittest.TestCase):
    def test_fixture_is_deterministic_and_preserves_nullable_columns(self):
        self.assertEqual(fact_row(0)[-1], "promo")
        self.assertIsNone(fact_row(1)[-1])
        with tempfile.TemporaryDirectory() as folder:
            path, oracle = make_fixture(folder, 17)
            self.assertEqual(path.read_text().splitlines()[0].split("\t")[-1], "coupon")
            self.assertIn("\\N", path.read_text())
            self.assertEqual(oracle.execute("SELECT count(*) FROM events").fetchone(), (17,))
            self.assertEqual(sqlite_reference(oracle, "SELECT COUNT(coupon) FROM events"), b"5\n")
            oracle.close()

    def test_invalid_rows_and_workloads_are_rejected(self):
        for rows in (0, 200_001):
            with self.assertRaises(ValueError):
                make_fixture("/unused", rows)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "invalid.json"
            for bad in ({"name": "x", "queries": []},
                        {"name": "x", "queries": [{"name": "q", "sql": "DELETE FROM events"}]},
                        {"name": "x", "queries": [
                            {"name": "q", "sql": "SELECT 1"}, {"name": "q", "sql": "SELECT 2"}]},):
                path.write_text(json.dumps(bad))
                with self.assertRaises(ValueError):
                    load_workload(path)

    def test_all_committed_workloads_are_sqlite_verifiable(self):
        with tempfile.TemporaryDirectory() as folder:
            _, oracle = make_fixture(folder, 97)
            try:
                paths = sorted(WORKLOADS.glob("*.json"))
                self.assertTrue(paths)
                for path in paths:
                    with self.subTest(workload=path.name):
                        for case in load_workload(path)["queries"]:
                            self.assertTrue(sqlite_reference(oracle, case["sql"]))
            finally:
                oracle.close()

    def test_bad_output_and_error_have_no_partial_latency(self):
        case = {"name": "q", "sql": "SELECT COUNT(*) FROM events"}
        self.assertEqual(benchmark_one(1, case, b"1\n", lambda _: b"2\n", 3, 1)["status"], "mismatch")
        self.assertEqual(benchmark_one(1, case, b"1\n", lambda _: b"2\n", 3, 1)["timings_seconds"], [])
        self.assertEqual(benchmark_one(1, case, b"1\n", lambda _: (_ for _ in ()).throw(RuntimeError("bad")), 2, 0)["status"], "error")

    def test_end_to_end_flow_checks_import_before_benchmark(self):
        workload = {"name": "basic", "queries": [
            {"name": "count", "sql": "SELECT COUNT(*) FROM events"}]}
        observed = []
        def fake_execute(sql):
            observed.append(sql)
            if sql.startswith("CREATE TABLE") or sql.startswith("COPY "):
                return b""
            return b"17\n"
        report = run_suite("rusthouse", workload, rows=17, runs=2, warmups=1, execute=fake_execute)
        self.assertEqual(report["coverage"]["verified_queries"], 1)
        self.assertEqual(len(report["results"][0]["timings_seconds"]), 2)
        self.assertEqual(sum(sql == "SELECT COUNT(*) FROM events" for sql in observed), 3)
        with self.assertRaisesRegex(ValueError, "fixture row count"):
            run_suite("rusthouse", workload, rows=17, execute=lambda _: b"wrong")


if __name__ == "__main__":
    unittest.main()

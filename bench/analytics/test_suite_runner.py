"""Oracle, fixture and execution contract tests for synthetic analytics."""
import json
import tempfile
import unittest
from pathlib import Path

from bench.analytics.suite_runner import (
    benchmark_one, fact_row, load_workload, make_fixture, run_suite,
    sqlite_reference, sqlite_typed_reference,
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
            self.assertEqual(oracle.execute("SELECT count(*) FROM accounts").fetchone(), (997,))
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
                            reference = (
                                sqlite_typed_reference(oracle, case["sql"])
                                if case.get("oracle", "exact") == "typed"
                                else sqlite_reference(oracle, case["sql"])
                            )
                            self.assertTrue(reference or case.get("oracle") == "typed")
            finally:
                oracle.close()

    def test_typed_oracle_supports_floats_nulls_and_exact_integers(self):
        with tempfile.TemporaryDirectory() as folder:
            _, oracle = make_fixture(folder, 17)
            expected = sqlite_typed_reference(
                oracle, "SELECT AVG(amount), MIN(amount) FROM events WHERE status = 'missing'"
            )
            self.assertEqual(expected, [(None, None)])
            oracle.close()
        case = {"name": "typed", "sql": "SELECT AVG(amount)", "oracle": "typed"}
        self.assertEqual(benchmark_one(1, case, [(2.0,)], lambda _: b"2.000000000000001\\n", 1, 0)["status"], "passed")
        self.assertEqual(benchmark_one(1, case, [(None,)], lambda _: b"NULL\\n", 1, 0)["status"], "passed")
        self.assertEqual(benchmark_one(1, case, [(9007199254740993,)],
                                       lambda _: b"9007199254740992\\n", 1, 0)["status"], "mismatch")
        unordered = {"name": "unordered", "sql": "SELECT user_id", "oracle": "typed", "unordered": True}
        self.assertEqual(benchmark_one(1, unordered, [(1,), (2,)],
                                       lambda _: b"2\\n1\\n", 1, 0)["status"], "passed")
        self.assertEqual(benchmark_one(1, unordered, [(1,), (2,)],
                                       lambda _: b"2\\n2\\n", 1, 0)["status"], "mismatch")

    def test_oracle_configuration_is_validated(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "cases.json"
            for case in (
                {"name": "q", "sql": "SELECT 1", "oracle": "invalid"},
                {"name": "q", "sql": "SELECT 1", "unordered": True},
                {"name": "q", "sql": "SELECT 1", "oracle": "typed", "unordered": "true"},
            ):
                path.write_text(json.dumps({"name": "suite", "queries": [case]}))
                with self.assertRaises(ValueError):
                    load_workload(path)

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
            return b"997\n" if sql == "SELECT count(*) FROM accounts" else b"17\n"
        report = run_suite("rusthouse", workload, rows=17, runs=2, warmups=1, execute=fake_execute)
        self.assertEqual(report["coverage"]["verified_queries"], 1)
        self.assertEqual(len(report["results"][0]["timings_seconds"]), 2)
        self.assertEqual(sum(sql == "SELECT COUNT(*) FROM events" for sql in observed), 3)
        with self.assertRaisesRegex(ValueError, "fixture row count"):
            run_suite("rusthouse", workload, rows=17, execute=lambda sql: b"997\n" if sql == "SELECT count(*) FROM accounts" else b"wrong")


    def test_sqlite_reference_limits_cursor_fetches(self):
        from unittest.mock import Mock
        cursor = Mock()
        cursor.fetchmany.return_value = [(7,)]
        oracle = Mock()
        oracle.execute.return_value = cursor
        self.assertEqual(sqlite_reference(oracle, "SELECT x"), b"7\n")
        self.assertEqual(sqlite_typed_reference(oracle, "SELECT x"), [(7,)])
        self.assertEqual(cursor.fetchmany.call_count, 2)
        self.assertEqual(cursor.fetchmany.call_args.args, (100_001,))
        cursor.fetchall.assert_not_called()

    def test_sqlite_reference_rejects_overlimit_without_fetchall(self):
        from unittest.mock import Mock
        cursor = Mock()
        cursor.fetchmany.return_value = [(1,)] * 100_001
        oracle = Mock()
        oracle.execute.return_value = cursor
        for build in (sqlite_reference, sqlite_typed_reference):
            with self.assertRaisesRegex(ValueError, "100000"):
                build(oracle, "SELECT x")
        cursor.fetchall.assert_not_called()


if __name__ == "__main__":
    unittest.main()

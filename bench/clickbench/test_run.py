import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("clickbench_runner", ROOT / "run.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class ClickBenchRunnerTests(unittest.TestCase):
    def test_pinned_queries_are_all_present(self):
        queries = runner.load_queries(ROOT / "queries.sql")
        self.assertEqual(len(queries), 43)
        self.assertEqual(queries[0], "SELECT COUNT(*) FROM hits")

    def test_query_selection_preserves_order_and_checks_ranges(self):
        self.assertEqual(runner.select_indices("3,1-2,2", 43), [0, 1, 2])
        with self.assertRaises(ValueError):
            runner.select_indices("0", 43)
        with self.assertRaises(ValueError):
            runner.select_indices("44", 43)
        with self.assertRaises(ValueError):
            runner.select_indices("4-2", 43)

    def test_unknown_output_is_never_marked_verified(self):
        results = runner.run_benchmark(["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"1\n")
        self.assertEqual(results[0]["status"], "unverified")
        self.assertEqual(len(results[0]["seconds"]), 3)

    def test_reference_mismatch_is_a_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "q01.tsv").write_bytes(b"expected\n")
            results = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"actual\n", root
            )
            self.assertEqual(results[0]["status"], "mismatch")
            self.assertEqual(len(results[0]["seconds"]), 1)

    def test_missing_reference_is_a_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            results = runner.run_benchmark(
                ["SELECT 1"], [0], 3, 1, lambda _sql, _timeout: b"1\n", Path(temp)
            )
            self.assertEqual(results[0]["status"], "missing_reference")

    def test_execution_error_is_recorded_without_hiding_other_queries(self):
        def execute(sql, _timeout):
            if sql == "bad":
                raise RuntimeError("unsupported")
            return b"good\n"

        results = runner.run_benchmark(["bad", "good"], [0, 1], 3, 1, execute)
        self.assertEqual([item["status"] for item in results], ["error", "unverified"])
        self.assertEqual(results[1]["seconds"].__len__(), 3)


if __name__ == "__main__":
    unittest.main()

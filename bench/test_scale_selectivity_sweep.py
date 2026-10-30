import unittest
from types import SimpleNamespace

from bench.scale.selectivity_sweep import run_sweep, thresholds


class SelectivitySweepTests(unittest.TestCase):
    def test_thresholds_include_both_boundaries(self):
        self.assertEqual(thresholds(8), [0, 2, 4, 6, 8])
        self.assertEqual(thresholds(1), [0, 1])

    def test_finer_selectivity_steps_preserve_exact_oracle(self):
        self.assertEqual(thresholds(10, steps=10), list(range(11)))
        self.assertEqual(thresholds(2, steps=64), [0, 1, 2])
        with self.assertRaisesRegex(ValueError, "steps"):
            thresholds(100, steps=65)
        def execute(command, **kwargs):
            threshold = int(command[2].rsplit(" ", 1)[-1])
            return SimpleNamespace(returncode=0,
                                   stdout=f"{10 - threshold}\n".encode(), stderr=b"")
        report = run_sweep("rusthouse", "part", 10, steps=10,
                           samples=1, execute=execute)
        self.assertEqual(report["steps"], 10)
        self.assertEqual(len(report["results"]), 11)

    def test_rejects_noninteger_and_out_of_range_rows(self):
        for rows in (True, 1.5, 0, -1, 2**64):
            with self.subTest(rows=rows):
                with self.assertRaisesRegex(ValueError, "UInt64"):
                    thresholds(rows)

    def test_verifies_each_predicate(self):
        def execute(command, **kwargs):
            threshold = int(command[2].rsplit(" ", 1)[-1])
            return SimpleNamespace(returncode=0, stdout=f"{8-threshold}\n".encode(), stderr=b"")
        report = run_sweep("rusthouse", "frozen", 8, samples=2, execute=execute)
        self.assertEqual(len(report["results"]), 5)
        self.assertTrue(all(result["verified"] for result in report["results"]))

    def test_fails_wrong_result(self):
        def execute(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"wrong\n", stderr=b"")
        with self.assertRaisesRegex(ValueError, "differs"):
            run_sweep("rusthouse", "frozen", 10, execute=execute)

    def test_rejects_unbounded_sweep_samples(self):
        from bench.scale.selectivity_sweep import run_sweep
        with self.assertRaisesRegex(ValueError, "samples"):
            run_sweep("rusthouse", "part", 1_000_000_000, samples=129)




class SelectivityExecutionBudgetTests(unittest.TestCase):
    def test_rejects_excessive_sql_runs_before_invoking_runner(self):
        calls = []
        with self.assertRaisesRegex(ValueError, "1024 SQL executions"):
            run_sweep("rusthouse", "part", 1_000_000_000, steps=64, samples=17,
                      execute=lambda *args, **kwargs: calls.append(args))
        self.assertEqual(calls, [])

if __name__ == "__main__":
    unittest.main()

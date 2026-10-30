import unittest
from types import SimpleNamespace

from bench.scale.selectivity_sweep import run_sweep, thresholds


class SelectivitySweepTests(unittest.TestCase):
    def test_output_budget_rejects_oversized_sql_results(self):
        from bench.scale.selectivity_sweep import execute_predicate
        def execute(*args, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"x" * 65, stderr=b"")
        with self.assertRaisesRegex(ValueError, "64 bytes"):
            execute_predicate(["fake"], 1, execute)

    def test_default_process_output_is_spooled(self):
        from unittest.mock import patch
        from bench.scale.selectivity_sweep import execute_predicate
        def run(_command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"1\n")
            kwargs["stderr"].write(b"x" * 10_000)
            return SimpleNamespace(returncode=0)
        with patch("bench.scale.selectivity_sweep.subprocess.run", side_effect=run):
            code, output, diagnostic = execute_predicate(["fake"], 1)
        self.assertEqual((code, output), (0, b"1\n"))
        self.assertEqual(len(diagnostic), 500)


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




class ShiftedSelectivityTests(unittest.TestCase):
    def test_shifted_sequential_key_domain(self):
        self.assertEqual(thresholds(8, steps=4, start=100), [100, 102, 104, 106, 108])

        def execute(command, **kwargs):
            threshold = int(command[2].rsplit(" ", 1)[-1])
            return SimpleNamespace(returncode=0,
                                   stdout=f"{108 - threshold}\n".encode(), stderr=b"")

        report = run_sweep("rusthouse", "part", 8, start=100, steps=4,
                           samples=1, execute=execute)
        self.assertEqual(report["start"], 100)
        self.assertEqual(report["results"][-1]["expected_count"], 0)
        with self.assertRaisesRegex(ValueError, "UInt64 start"):
            thresholds(8, start=2**64 - 8)


class SelectivityExecutionBudgetTests(unittest.TestCase):
    def test_rejects_excessive_sql_runs_before_invoking_runner(self):
        calls = []
        with self.assertRaisesRegex(ValueError, "1024 SQL executions"):
            run_sweep("rusthouse", "part", 1_000_000_000, steps=64, samples=17,
                      execute=lambda *args, **kwargs: calls.append(args))
        self.assertEqual(calls, [])

if __name__ == "__main__":
    unittest.main()

"""Predicate sweeps must bound child output before buffering to disk."""
import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.selectivity_sweep import MAX_RESULT_BYTES, execute_predicate


class PredicateOutputBudgetTests(unittest.TestCase):
    @patch("bench.scale.selectivity_sweep.subprocess.run")
    def test_child_has_online_file_size_limit(self, run):
        def execute(command, **kwargs):
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            else:
                self.assertNotIn("preexec_fn", kwargs)
            kwargs["stdout"].write(b"1\n")
            return SimpleNamespace(returncode=0)
        run.side_effect = execute
        self.assertEqual(execute_predicate(["rusthouse", "sql"], 3)[:2], (0, b"1\n"))

    @patch("bench.scale.selectivity_sweep.subprocess.run")
    def test_rejects_oversized_child_output(self, run):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"0" * (MAX_RESULT_BYTES + 1))
            return SimpleNamespace(returncode=0)
        run.side_effect = execute
        with self.assertRaisesRegex(ValueError, "64 bytes"):
            execute_predicate(["rusthouse", "sql"], 3)

    def test_injected_runner_contract_is_unchanged(self):
        def runner(command, **kwargs):
            self.assertEqual(kwargs["timeout"], 1.5)
            self.assertTrue(kwargs["capture_output"])
            return SimpleNamespace(returncode=0, stdout=b"7\n", stderr=b"")
        self.assertEqual(execute_predicate(["fake"], 1.5, runner=runner)[:2],
                         (0, b"7\n"))


if __name__ == "__main__":
    unittest.main()

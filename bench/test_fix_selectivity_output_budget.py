import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.selectivity_sweep import execute_predicate


class SelectivityOutputBudgetTests(unittest.TestCase):
    def test_posix_child_has_bounded_file_output(self):
        def run(_command, **kwargs):
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            else:
                self.assertNotIn("preexec_fn", kwargs)
            kwargs["stdout"].write(b"42\n")
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.selectivity_sweep.subprocess.run", side_effect=run):
            code, data, diagnostic = execute_predicate(["binary"], 30)
        self.assertEqual((code, data, diagnostic), (0, b"42\n", b""))

    def test_rejects_oversized_child_response(self):
        def run(_command, **kwargs):
            kwargs["stdout"].write(b"x" * 65)
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.selectivity_sweep.subprocess.run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "64 bytes"):
                execute_predicate(["binary"], 30)


if __name__ == "__main__":
    unittest.main()

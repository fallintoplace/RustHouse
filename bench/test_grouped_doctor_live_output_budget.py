"""A stuck or verbose doctor must not fill the scratch disk."""
import os
import subprocess
import unittest
from unittest.mock import patch

from bench.scale.grouped_physical import inspect_persisted_snapshot


class OnlineDoctorOutputBudgetTests(unittest.TestCase):
    @patch("bench.scale.grouped_physical.subprocess.run")
    def test_success_uses_online_cap_on_posix(self, run):
        def inspect(command, **kwargs):
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            else:
                self.assertNotIn("preexec_fn", kwargs)
            kwargs["stdout"].write(b"OK events: 4 rows, 1 parts\n")
            return subprocess.CompletedProcess(command, 0)
        run.side_effect = inspect
        inspect_persisted_snapshot("rusthouse", "db", 4, 5)

    @patch("bench.scale.grouped_physical.subprocess.run")
    def test_oversized_doctor_output_still_fails(self, run):
        def inspect(command, **kwargs):
            kwargs["stdout"].write(b"x" * (256 * 1024 + 1))
            return subprocess.CompletedProcess(command, 0)
        run.side_effect = inspect
        with self.assertRaisesRegex(RuntimeError, "256 KiB"):
            inspect_persisted_snapshot("rusthouse", "db", 4, 5)


if __name__ == "__main__":
    unittest.main()

import subprocess
import unittest
from unittest.mock import patch

from bench.scale.grouped_physical import inspect_persisted_snapshot


class GroupedDoctorOutputTests(unittest.TestCase):
    def test_successful_snapshot_verification(self):
        def run(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"OK events: 10 rows, 1 parts\n")
            return subprocess.CompletedProcess(command, 0)
        with patch("bench.scale.grouped_physical.subprocess.run", side_effect=run):
            inspect_persisted_snapshot("rusthouse", "db", 10, 2)

    def test_doctor_summary_must_begin_on_its_own_line(self):
        def run(command, **kwargs):
            kwargs["stdout"].write(
                b"WARN: previously OK events: 10 rows, 1 parts\n"
                b"NOT OK events: 10 rows, 1 parts\n"
            )
            return subprocess.CompletedProcess(command, 0)
        with patch("bench.scale.grouped_physical.subprocess.run", side_effect=run):
            with self.assertRaisesRegex(RuntimeError, "verification failed"):
                inspect_persisted_snapshot("rusthouse", "db", 10, 2)

    def test_doctor_summary_can_follow_other_complete_lines(self):
        def run(command, **kwargs):
            kwargs["stdout"].write(
                b"OK unrelated: 99 rows, 1 parts\n"
                b"OK events: 10 rows, 1 parts\n"
            )
            return subprocess.CompletedProcess(command, 0)
        with patch("bench.scale.grouped_physical.subprocess.run", side_effect=run):
            inspect_persisted_snapshot("rusthouse", "db", 10, 2)

    def test_failed_snapshot_shows_bounded_final_error(self):
        def run(command, **kwargs):
            kwargs["stderr"].write(b"x" * 100_000 + b"fatal error")
            return subprocess.CompletedProcess(command, 2)
        with patch("bench.scale.grouped_physical.subprocess.run", side_effect=run):
            with self.assertRaises(RuntimeError) as caught:
                inspect_persisted_snapshot("rusthouse", "db", 10, 2)
        self.assertIn("fatal error", str(caught.exception))
        self.assertLess(len(str(caught.exception)), 1000)

    def test_oversized_doctor_output_fails_without_full_read(self):
        def run(command, **kwargs):
            kwargs["stdout"].truncate(256 * 1024 + 1)
            return subprocess.CompletedProcess(command, 0)
        with patch("bench.scale.grouped_physical.subprocess.run", side_effect=run):
            with self.assertRaisesRegex(RuntimeError, "256 KiB"):
                inspect_persisted_snapshot("rusthouse", "db", 10, 2)


if __name__ == "__main__":
    unittest.main()

import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from bench.scale.copy_disk_pressure_guard import guard, sql_for


class DiskGuardTests(unittest.TestCase):
    def test_escapes_apostrophes_and_checks_table_identifiers(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "o'brien.tsv"
            path.touch()
            self.assertIn("o''brien.tsv", sql_for("events", path))
            with self.assertRaises(ValueError):
                sql_for("events; DROP TABLE", path)

    def test_large_stderr_does_not_deadlock_copy_guard(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "data.tsv"
            source.write_text("1\n")
            real_popen = subprocess.Popen
            script = "import sys; sys.stderr.write('X' * 262144); sys.exit(17)"

            def launch(_command, **kwargs):
                return real_popen([sys.executable, "-c", script], **kwargs)

            with patch("bench.scale.copy_disk_pressure_guard.subprocess.Popen", side_effect=launch):
                with self.assertRaisesRegex(RuntimeError, "COPY failed"):
                    guard("fake", "db", "events", source, temp,
                          min_free_gib=0.000001, timeout=3, poll_seconds=0.05)

    def test_refuses_missing_input_and_bad_budgets(self):
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "missing.tsv"
            with self.assertRaises(ValueError):
                guard("unused", "unused", "events", fake, temp)
            fake.touch()
            with self.assertRaisesRegex(ValueError, "min-free"):
                guard("unused", "unused", "events", fake, temp, min_free_gib=0)
            with self.assertRaisesRegex(ValueError, "poll"):
                guard("unused", "unused", "events", fake, temp,
                      poll_seconds=0.001)

    def test_rejects_monitor_on_another_existing_database_device(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "input.tsv"
            source.write_bytes(b"1\n")
            db = root / "db"
            db.mkdir()
            with patch("bench.scale.copy_disk_pressure_guard.same_device", return_value=False):
                with self.assertRaisesRegex(ValueError, "database filesystem"):
                    guard("rusthouse", db, "events", source, root,
                          min_free_gib=0.000001)


if __name__ == "__main__":
    unittest.main()

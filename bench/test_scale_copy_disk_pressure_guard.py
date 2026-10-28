import tempfile
import unittest
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


if __name__ == "__main__":
    unittest.main()

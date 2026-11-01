import tempfile
import unittest
from pathlib import Path
from bench.scale.part_density import part_density


class PartDensityDirectoryBudgets(unittest.TestCase):
    def test_empty_unrelated_entries_are_still_counted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            part = root / "part-1"
            part.mkdir()
            (part / "count.txt").write_bytes(b"12\n")
            (root / "unused-1").mkdir()
            (root / "unused-2").mkdir()
            with self.assertRaisesRegex(ValueError, "max_entries"):
                part_density(root, max_entries=2)
            report = part_density(root, max_entries=3)
            self.assertEqual(report["part_count"], 1)
            self.assertEqual(report["rows"], 12)

    def test_rejects_invalid_scanning_budgets(self):
        with tempfile.TemporaryDirectory() as directory:
            for value in (True, -1, 0, 1.5):
                with self.subTest(value=value), self.assertRaisesRegex(ValueError, "max_entries"):
                    part_density(directory, max_entries=value)


if __name__ == "__main__":
    unittest.main()

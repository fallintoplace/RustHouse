import tempfile
import unittest
from pathlib import Path
from bench.scale.snapshot_bytes import inventory


class DirectoryBudgetInventoryTests(unittest.TestCase):
    def test_limits_empty_directories_in_addition_to_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a" / "b").mkdir(parents=True)
            (root / "data.bin").write_bytes(b"abc")
            with self.assertRaisesRegex(ValueError, "max_dirs"):
                inventory(root, max_files=10, max_dirs=2)
            report = inventory(root, max_files=10, max_dirs=3)
            self.assertEqual(report["file_count"], 1)
            self.assertEqual(report["total_bytes"], 3)

    def test_rejects_invalid_directory_budgets(self):
        with tempfile.TemporaryDirectory() as directory:
            for value in (0, -1, True, 1.5):
                with self.subTest(value=value), self.assertRaisesRegex(ValueError, "max_dirs"):
                    inventory(Path(directory), max_dirs=value)


if __name__ == "__main__":
    unittest.main()

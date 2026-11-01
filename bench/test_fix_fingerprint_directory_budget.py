import tempfile
import unittest
from pathlib import Path

from bench.scale.snapshot_fingerprint import fingerprint


class FingerprintDirectoryBudgetTests(unittest.TestCase):
    def test_counts_directories_even_when_they_are_empty(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "a").mkdir()
            (directory / "b").mkdir()
            (directory / "data.bin").write_bytes(b"payload")
            with self.assertRaisesRegex(ValueError, "max_dirs"):
                fingerprint(directory, max_dirs=2)
            self.assertEqual(fingerprint(directory, max_dirs=3)["files"], 1)

    def test_rejects_invalid_directory_budget_before_walking(self):
        for invalid in (0, -1, True, 1.5, "3"):
            with self.subTest(value=invalid), self.assertRaisesRegex(ValueError, "max_dirs"):
                fingerprint("unused-path", max_dirs=invalid)


if __name__ == "__main__":
    unittest.main()

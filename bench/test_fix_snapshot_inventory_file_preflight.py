import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.snapshot_bytes import inventory


class SnapshotInventoryFilePreflightTests(unittest.TestCase):
    def test_rejects_second_file_before_statting_it(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "a.bin").write_bytes(b"a")
            (directory / "b.bin").write_bytes(b"b")
            original_lstat = Path.lstat
            examined = []

            def checked_lstat(path):
                if path.name in ("a.bin", "b.bin"):
                    examined.append(path.name)
                    if len(examined) > 1:
                        raise AssertionError("an extra file must not be examined")
                return original_lstat(path)

            with patch.object(Path, "lstat", checked_lstat):
                with self.assertRaisesRegex(ValueError, "max_files"):
                    inventory(directory, max_files=1)
            self.assertEqual(len(examined), 1)

    def test_exact_budget_still_counts_every_file(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            (directory / "a.bin").write_bytes(b"a")
            (directory / "b.mrk2").write_bytes(b"xy")
            report = inventory(directory, max_files=2)
            self.assertEqual(report["file_count"], 2)
            self.assertEqual(report["total_bytes"], 3)


if __name__ == "__main__":
    unittest.main()

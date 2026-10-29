"""Physical snapshot byte inventories require real files."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.snapshot_bytes import inventory


class SnapshotByteInventoryTests(unittest.TestCase):
    def test_empty_snapshot_is_not_reported_as_valid(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "no files"):
                inventory(root)

    def test_valid_snapshot_still_reports_size_and_largest_file(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "id.bin").write_bytes(b"1234")
            report = inventory(root, largest_files=1)
            self.assertEqual(report["total_bytes"], 4)
            self.assertEqual(report["file_count"], 1)
            self.assertEqual(report["largest_files"], [{"path": "id.bin", "bytes": 4}])

    def test_boolean_file_limit_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "positive integer"):
                inventory(root, max_files=True)

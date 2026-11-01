"""Logical and unique-inode snapshot sizes must not be confused."""
import os
import tempfile
import unittest
from pathlib import Path

from bench.scale.snapshot_bytes import inventory


class HardlinkFootprintTests(unittest.TestCase):
    def test_reports_unique_inodes_without_changing_logical_groups(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            first = root / "a.bin"
            first.write_bytes(b"x" * 101)
            try:
                os.link(first, root / "b.bin")
            except (OSError, NotImplementedError):
                self.skipTest("hardlinks not supported on this filesystem")
            (root / "metadata.txt").write_bytes(b"hi")
            result = inventory(root, deduplicate_hardlinks=True)
            self.assertEqual(result["total_bytes"], 204)
            self.assertEqual(result["byte_groups"]["data_bytes"], 202)
            self.assertEqual(result["unique_file_bytes"], 103)
            self.assertEqual(result["hardlinked_alias_bytes"], 101)
            self.assertEqual(result["unique_inodes"], 2)
            self.assertEqual(inventory(root)["total_bytes"], result["total_bytes"])
            self.assertNotIn("unique_file_bytes", inventory(root))
            with self.assertRaisesRegex(ValueError, "max_total_bytes"):
                inventory(root, max_total_bytes=103, deduplicate_hardlinks=True)

    def test_boolean_flag_is_strict(self):
        with self.assertRaisesRegex(ValueError, "deduplicate_hardlinks"):
            inventory(".", deduplicate_hardlinks=1)


if __name__ == "__main__":
    unittest.main()

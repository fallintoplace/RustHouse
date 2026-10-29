import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from bench.scale.snapshot_bytes import classify, inventory


class SnapshotBytesTests(unittest.TestCase):
    def test_groups_data_marks_and_index_files(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "x.bin").write_bytes(b"a" * 10)
            (Path(root) / "x.mrk2").write_bytes(b"a" * 3)
            (Path(root) / "primary.idx").write_bytes(b"a" * 2)
            (Path(root) / "count.txt").write_bytes(b"10\n")
            result = inventory(root)
            self.assertEqual(result["byte_groups"]["data_bytes"], 10)
            self.assertEqual(result["byte_groups"]["mark_bytes"], 3)
            self.assertEqual(result["byte_groups"]["index_bytes"], 2)
            self.assertEqual(result["total_bytes"], 18)
            self.assertEqual(result["file_count"], 4)

    def test_rejects_excessive_file_count(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "x.bin").write_bytes(b"a")
            with self.assertRaisesRegex(ValueError, "max_files"):
                inventory(root, max_files=0)

    def test_rejects_unreadable_subdirectories(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "data.bin").write_bytes(b"123")

            def unreadable(_root, *, followlinks, onerror):
                onerror(PermissionError("unreadable nested part"))
                return iter(())

            with patch("bench.scale.snapshot_bytes.os.walk", side_effect=unreadable):
                with self.assertRaisesRegex(PermissionError, "unreadable"):
                    inventory(root)

    def test_classifies_unknown_files_as_metadata(self):
        self.assertEqual(classify("columns.txt"), "metadata_bytes")

    def test_compressed_marks_are_mark_overhead(self):
        from bench.scale.snapshot_bytes import classify
        self.assertEqual(classify("id.cmrk2"), "mark_bytes")
        self.assertEqual(classify("id.cmrk3"), "mark_bytes")
        self.assertEqual(classify("id.bin"), "data_bytes")


if __name__ == "__main__":
    unittest.main()

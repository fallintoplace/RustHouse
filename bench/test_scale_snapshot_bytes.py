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

    def test_largest_files_are_bounded_and_sorted(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "nested"
            folder.mkdir()
            (Path(root) / "a.bin").write_bytes(b"a" * 3)
            (folder / "big.bin").write_bytes(b"b" * 20)
            (folder / "mid.bin").write_bytes(b"c" * 8)
            result = inventory(root, largest_files=2)
            self.assertEqual(result["largest_files"], [
                {"path": "nested/big.bin", "bytes": 20},
                {"path": "nested/mid.bin", "bytes": 8},
            ])
            self.assertEqual(result["total_bytes"], 31)
            self.assertNotIn("largest_files", inventory(root))
            with self.assertRaisesRegex(ValueError, "largest_files"):
                inventory(root, largest_files=101)

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


    def test_requires_integer_file_limit(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "a.bin").write_bytes(b"x")
            for invalid in (True, 2.5, float("nan"), "3", 0):
                with self.subTest(value=invalid), self.assertRaisesRegex(ValueError, "max_files"):
                    inventory(root, max_files=invalid)

if __name__ == "__main__":
    unittest.main()

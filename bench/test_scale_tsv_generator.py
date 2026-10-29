import io
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_generator import MAX_UINT64, emit_rows, generate


class MassiveTsvGeneratorTests(unittest.TestCase):
    def test_bounded_batches_keep_sequence_and_header(self):
        output = io.StringIO()
        self.assertEqual(emit_rows(output, 5, start=10, batch_rows=2, header=True), 5)
        self.assertEqual(output.getvalue(), "id\n10\n11\n12\n13\n14\n")

    def test_can_start_above_32_bit_boundary(self):
        output = io.StringIO()
        emit_rows(output, 2, start=(1 << 32), batch_rows=1)
        self.assertEqual(output.getvalue(), "4294967296\n4294967297\n")

    def test_checks_uint64_overflow_before_writing(self):
        output = io.StringIO()
        with self.assertRaisesRegex(ValueError, "overflow"):
            emit_rows(output, 2, start=MAX_UINT64)
        self.assertEqual(output.getvalue(), "")

    def test_rejects_invalid_batch(self):
        with self.assertRaisesRegex(ValueError, "batch_rows"):
            emit_rows(io.StringIO(), 1, batch_rows=0)

    def test_publish_does_not_overwrite_without_force(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "huge.tsv"
            generate(path, 3)
            self.assertEqual(path.read_text(), "0\n1\n2\n")
            with self.assertRaises(FileExistsError):
                generate(path, 2)
            generate(path, 2, start=5, force=True)
            self.assertEqual(path.read_text(), "5\n6\n")
            self.assertEqual(list(Path(root).glob("*.tmp")), [])

    def test_dangling_symlink_is_not_overwritten(self):
        import sys
        if sys.platform == "win32":
            self.skipTest("symlinks are not available on Windows CI")
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "huge.tsv"
            path.symlink_to(Path(root) / "missing.tsv")
            with self.assertRaises(FileExistsError):
                generate(path, 3)
            self.assertTrue(path.is_symlink())


if __name__ == "__main__":
    unittest.main()

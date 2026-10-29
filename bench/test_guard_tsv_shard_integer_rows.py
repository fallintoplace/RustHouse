import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_shards import split_tsv


class ShardIntegerBoundsTests(unittest.TestCase):
    def test_rejects_fractional_and_boolean_shard_limits_before_writing(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_bytes(b"1\n2\n")
            target = Path(root) / "chunks"
            for invalid in (True, 1.5, 0.5, False):
                with self.subTest(limit=invalid), self.assertRaisesRegex(
                        ValueError, "rows_per_chunk"):
                    split_tsv(source, target, rows_per_chunk=invalid)
                self.assertFalse(target.exists())

    def test_integer_row_limit_still_creates_correct_chunks(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.tsv"
            source.write_bytes(b"1\n2\n3\n")
            report = split_tsv(source, Path(root) / "chunks", rows_per_chunk=2)
            self.assertEqual([entry["rows"] for entry in report["shards"]], [2, 1])


if __name__ == "__main__":
    unittest.main()

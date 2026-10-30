"""TSV shard sizing cannot silently interpret booleans as chunk sizes."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_shards import split_tsv


class TsvShardChunkSizeTests(unittest.TestCase):
    def test_boolean_chunk_size_fails_before_creating_shards(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "input.tsv"
            source.write_bytes(b"1\n2\n")
            output = root / "shards"
            with self.assertRaisesRegex(ValueError, "rows_per_chunk"):
                split_tsv(source, output, rows_per_chunk=True)
            self.assertFalse(output.exists())

    def test_integer_chunk_size_keeps_exact_records(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "input.tsv"
            source.write_bytes(b"1\n2\n")
            output = root / "shards"
            report = split_tsv(source, output, rows_per_chunk=1)
            self.assertEqual(report["total_rows"], 2)
            self.assertEqual((output / "part-000000.tsv").read_bytes(), b"1\n")
            self.assertEqual((output / "part-000001.tsv").read_bytes(), b"2\n")

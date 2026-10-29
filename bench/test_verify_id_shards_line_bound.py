"""Regression tests for bounded UInt64 shard line reads."""
import hashlib
import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import verify_file


class VerifyIdShardLineTests(unittest.TestCase):
    def test_accepts_maximum_uint64_line(self):
        value = (1 << 64) - 1
        payload = f"{value}\n".encode("ascii")
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "shard.tsv"
            path.write_bytes(payload)
            self.assertEqual(verify_file(path, value, 1), hashlib.sha256(payload).hexdigest())

    def test_rejects_oversized_unterminated_line(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "shard.tsv"
            path.write_bytes(b"9" * 200_000)
            with self.assertRaisesRegex(ValueError, "incorrect or missing"):
                verify_file(path, 0, 1)

    def test_rejects_trailing_data(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "shard.tsv"
            path.write_bytes(b"0\n1\n")
            with self.assertRaisesRegex(ValueError, "additional rows"):
                verify_file(path, 0, 1)

"""Preflight UInt64 shard byte counts without iterating manifest rows."""
import hashlib
import random
import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import expected_shard_bytes, verify_file


class ExactShardSizeTests(unittest.TestCase):
    def test_digit_transitions_match_serialized_fixtures(self):
        randomizer = random.Random(27)
        cases = [(0, 1, 1), (9, 9, 1), (98, 9, 1),
                 (999_999, 20, 17),
                 ((1 << 64) - 3, 2, 2)]
        for _ in range(100):
            cases.append((randomizer.randrange(100_000),
                          randomizer.randrange(1, 300),
                          randomizer.randrange(1, 30)))
        for start, rows, step in cases:
            expected = "".join(f"{start + i * step}\n" for i in range(rows)).encode()
            with self.subTest(start=start, rows=rows, step=step):
                self.assertEqual(expected_shard_bytes(start, rows, step), len(expected))

    def test_huge_manifest_rejected_before_generating_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            shard = Path(directory) / "data.tsv"
            shard.write_bytes(b"0\n")
            with self.assertRaisesRegex(ValueError, "incorrect or missing"):
                verify_file(shard, 0, 1_000_000_000_000, 1)

    def test_exact_sizes_still_hash_and_verify_contents(self):
        with tempfile.TemporaryDirectory() as directory:
            shard = Path(directory) / "data.tsv"
            shard.write_bytes(b"8\n10\n12\n")
            expected = hashlib.sha256(shard.read_bytes()).hexdigest()
            self.assertEqual(verify_file(shard, 8, 3, 2), expected)
            shard.write_bytes(b"8\n11\n12\n")
            with self.assertRaisesRegex(ValueError, "incorrect"):
                verify_file(shard, 8, 3, 2)
            shard.write_bytes(b"8\n10\n12\n14\n")
            with self.assertRaisesRegex(ValueError, "additional"):
                verify_file(shard, 8, 3, 2)


if __name__ == "__main__":
    unittest.main()

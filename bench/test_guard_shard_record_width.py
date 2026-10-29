import tempfile
import unittest
from pathlib import Path

from bench.scale.verify_id_shards import verify_file


class BoundedShardRecordTests(unittest.TestCase):
    def test_valid_20_digit_uint64_line(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "rows.tsv"
            path.write_bytes(b"18446744073709551615\n")
            self.assertEqual(len(verify_file(path, (1 << 64) - 1, 1)), 64)

    def test_rejects_overlong_or_truncated_records(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "rows.tsv"
            for payload in (b"0" * 100_000 + b"\n", b"1" * 21 + b"\n", b"0"):
                path.write_bytes(payload)
                with self.subTest(length=len(payload)), self.assertRaisesRegex(
                        ValueError, "incorrect or missing row"):
                    verify_file(path, 0, 1)


if __name__ == "__main__":
    unittest.main()

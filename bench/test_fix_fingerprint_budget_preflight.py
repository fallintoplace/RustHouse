import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.snapshot_fingerprint import fingerprint


class FingerprintBudgetTests(unittest.TestCase):
    def test_max_files_rejects_before_opening_extra_file(self):
        with tempfile.TemporaryDirectory() as root:
            first = Path(root) / "a.bin"
            second = Path(root) / "b.bin"
            first.write_bytes(b"abc")
            second.write_bytes(b"def")
            real_open = os.open

            def checked_open(path, *args, **kwargs):
                if Path(path) == second:
                    self.fail("second file must not be opened")
                return real_open(path, *args, **kwargs)

            with patch("bench.scale.snapshot_fingerprint.os.open", side_effect=checked_open):
                with self.assertRaisesRegex(ValueError, "max_files"):
                    fingerprint(root, max_files=1, block_size=4096)

    def test_total_bytes_preflight_rejects_before_hashing_extra_file(self):
        with tempfile.TemporaryDirectory() as root:
            first = Path(root) / "a.bin"
            second = Path(root) / "b.bin"
            first.write_bytes(b"abc")
            second.write_bytes(b"def")
            real_open = os.open

            def checked_open(path, *args, **kwargs):
                if Path(path) == second:
                    self.fail("second file must not be opened")
                return real_open(path, *args, **kwargs)

            with patch("bench.scale.snapshot_fingerprint.os.open", side_effect=checked_open):
                with self.assertRaisesRegex(ValueError, "max_total_bytes"):
                    fingerprint(root, max_total_bytes=5, block_size=4096)
            self.assertEqual(fingerprint(root, max_total_bytes=6, block_size=4096)["bytes"], 6)


if __name__ == "__main__":
    unittest.main()

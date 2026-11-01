"""Latency report readers reject path replacement and nonstandard JSON."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.latency import validate_report


class StrictLatencyFileTests(unittest.TestCase):
    def test_rejects_nonstandard_constants(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_bytes(b'{"suite":"generated-sql-latency","other":NaN}')
            with self.assertRaisesRegex(ValueError, "nonstandard latency JSON"):
                validate_report.load_report(path)

    def test_detects_file_swaps_during_open(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            swap = Path(root) / "swap.json"
            path.write_bytes(b'{"suite":"good"}')
            swap.write_bytes(b'{"suite":"swapped"}')
            original = os.open
            def replace(*args, **kwargs):
                os.replace(swap, path)
                return original(*args, **kwargs)
            with patch.object(validate_report.os, "open", side_effect=replace):
                with self.assertRaisesRegex(ValueError, "changed before opening"):
                    validate_report.load_report(path)


if __name__ == "__main__":
    unittest.main()

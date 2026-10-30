"""Streaming numeric TSV validation has an ASCII-only fast path."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_profile import profile


class SortedTsvProfileTests(unittest.TestCase):
    def test_validates_ordered_uint64_with_header(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sorted.tsv"
            path.write_bytes(b"id\n0\n12\n18446744073709551615\n")
            report = profile(path, header=True, verify_sorted_uint64=True)
            self.assertEqual(report["rows"], 3)
            self.assertEqual(report["columns"], 1)

    def test_rejects_invalid_numeric_bytes(self):
        for bad in (b"0\n\xff\n", b"0\n0\n", b"0\n18446744073709551616\n"):
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / "bad.tsv"
                path.write_bytes(bad)
                with self.assertRaises(ValueError):
                    profile(path, verify_sorted_uint64=True)

    def test_generic_input_still_requires_utf8(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "bad.tsv"
            path.write_bytes(b"\xff\n")
            with self.assertRaises(UnicodeError):
                profile(path)

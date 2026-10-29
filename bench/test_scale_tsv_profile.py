import hashlib
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_profile import profile


class InputProfileTests(unittest.TestCase):
    def test_hash_and_data_rows_exclude_header(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            raw = b"id\tlabel\n1\ta\n2\tb\n"
            path.write_bytes(raw)
            result = profile(path, columns=2, header=True, min_rows=2)
            self.assertEqual(result["rows"], 2)
            self.assertEqual(result["bytes"], len(raw))
            self.assertEqual(result["sha256"], hashlib.sha256(raw).hexdigest())

    def test_rejects_bad_column_counts(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            path.write_text("a\tb\nc\n")
            with self.assertRaisesRegex(ValueError, "column"):
                profile(path, columns=2)

    def test_rejects_unterminated_line(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            path.write_bytes(b"42")
            with self.assertRaisesRegex(ValueError, "truncated"):
                profile(path)

    def test_rejects_invalid_utf8(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            path.write_bytes(b"\xff\n")
            with self.assertRaises(UnicodeError):
                profile(path)

    def test_minimum_rows_gate(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "events.tsv"
            path.write_bytes(b"1\n")
            with self.assertRaisesRegex(ValueError, "require 2"):
                profile(path, min_rows=2)

    def test_rejects_raw_nul_but_allows_tsv_null_escape(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "data.tsv"
            path.write_bytes(b"bad\x00value\n")
            with self.assertRaisesRegex(ValueError, "NUL"):
                profile(path)
            path.write_bytes(b"\\N\n")
            self.assertEqual(profile(path)["rows"], 1)


if __name__ == "__main__":
    unittest.main()

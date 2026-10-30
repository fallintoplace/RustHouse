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


    def test_sorted_integer_profile_requires_canonical_decimal_keys(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "keys.tsv"
            for payload in (b"00\n1\n", b"01\n2\n", b"0\n0002\n"):
                with self.subTest(payload=payload):
                    source.write_bytes(payload)
                    with self.assertRaisesRegex(ValueError, "invalid UInt64"):
                        profile(source, verify_sorted_uint64=True)
            source.write_bytes(b"0\n1\n10\n")
            self.assertEqual(profile(source, verify_sorted_uint64=True)["rows"], 3)

    def test_optional_sorted_uint64_verification(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "keys.tsv"
            path.write_bytes(b"id\n0\n3\n18446744073709551615\n")
            result = profile(path, header=True, verify_sorted_uint64=True)
            self.assertEqual(result["rows"], 3)
            path.write_bytes(b"id\n3\n3\n")
            with self.assertRaisesRegex(ValueError, "strictly increasing"):
                profile(path, header=True, verify_sorted_uint64=True)
            path.write_bytes(b"18446744073709551616\n")
            with self.assertRaisesRegex(ValueError, "overflow"):
                profile(path, verify_sorted_uint64=True)


if __name__ == "__main__":
    unittest.main()

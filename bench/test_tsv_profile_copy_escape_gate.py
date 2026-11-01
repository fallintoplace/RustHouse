"""A TSV profile may optionally verify full COPY escape compatibility."""
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_profile import profile


class CopyProfileTests(unittest.TestCase):
    def test_valid_escaped_utf8_and_null_fields(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "rows.tsv"
            path.write_bytes(b"key\tmessage\n1\tcaf\\xC3\\xA9\n2\t\\N\n")
            result = profile(path, columns=2, header=True,
                             verify_copy_escapes=True, min_rows=2)
            self.assertEqual(result["rows"], 2)
            self.assertTrue(result["copy_escapes_verified"])
            self.assertNotIn("copy_escapes_verified", profile(
                path, columns=2, header=True))

    def test_fails_on_invalid_escape_or_decoded_utf8_only_in_strict_mode(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "rows.tsv"
            for payload in (b"bad\\q\n", b"bad\\xFF\n", b"bad\\xC3\n"):
                with self.subTest(payload=payload):
                    path.write_bytes(payload)
                    self.assertEqual(profile(path)["rows"], 1)
                    with self.assertRaises(ValueError):
                        profile(path, verify_copy_escapes=True)

    def test_header_is_not_treated_as_copy_data(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "rows.tsv"
            path.write_bytes(b"bad\\q\nvalid\n")
            self.assertTrue(profile(path, header=True,
                                    verify_copy_escapes=True)["copy_escapes_verified"])

    def test_column_and_flag_validation_are_strict(self):
        for options in (
            {"columns": 129, "verify_copy_escapes": True},
            {"verify_copy_escapes": 1},
        ):
            with self.subTest(options=options), self.assertRaisesRegex(
                ValueError, "verify_copy_escapes"
            ):
                profile("unopened.tsv", **options)


if __name__ == "__main__":
    unittest.main()

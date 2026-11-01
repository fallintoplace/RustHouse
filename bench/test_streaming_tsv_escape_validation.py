"""Streaming TSV escape validation must preserve UTF-8 semantics."""
import unittest

from bench.scale.tsv_shards import validate_copy_tsv_record


class IncrementalEscapedFieldTests(unittest.TestCase):
    def test_utf8_codepoint_split_across_hex_escapes(self):
        for valid in (
            b"caf\\xC3\\xA9\n",
            b"\\xF0\\x9F\\x92\\xBE\n",
            b"\\xE2\\x82\\xAC\\n\n",
            b"plain\\tfield\n",
            b"\\x00\n",
        ):
            with self.subTest(valid=valid):
                validate_copy_tsv_record(valid)

    def test_rejects_incomplete_and_invalid_multibyte_sequences(self):
        for bad in (
            b"\\xC3\n", b"\\xC3\\x28\n",
            b"\\xF0\\x9F\\x92\n", b"\\x80\n",
            b"\\xFF\n", b"\\xGG\n", b"ending\\\n",
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                validate_copy_tsv_record(bad)

    def test_long_plain_spans_survive_incremental_utf8_boundaries(self):
        record = (b"a" * (1024 * 1024) + b"\\xF0\\x9F\\x92\\xBE"
                  + b"b" * (1024 * 1024) + b"\n")
        validate_copy_tsv_record(record)
        with self.assertRaises(ValueError):
            validate_copy_tsv_record(record[:-1] + b"\\x80\n")


if __name__ == "__main__":
    unittest.main()

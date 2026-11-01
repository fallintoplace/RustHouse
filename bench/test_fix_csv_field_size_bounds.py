"""Oversized CSV limits must fail before mutating global parser state."""
import csv
import tempfile
import unittest
from pathlib import Path

from bench.scale.csv_stream_to_tsv import convert


class CsvAbsoluteFieldLimitTests(unittest.TestCase):
    def test_rejects_limits_that_cannot_fit_copy_rows(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.csv"
            source.write_text("name\nhello\n", encoding="utf-8")
            original_limit = csv.field_size_limit()
            for limit in (0, True, 8 * 1024 * 1024 + 1, 10 ** 100):
                with self.subTest(limit=limit):
                    output = Path(folder) / "result.tsv"
                    with self.assertRaisesRegex(ValueError, "max_field_bytes"):
                        convert(source, output, header=True, max_field_bytes=limit)
                    self.assertFalse(output.exists())
                    self.assertEqual(csv.field_size_limit(), original_limit)
            self.assertEqual(convert(
                source, Path(folder) / "valid.tsv", header=True, max_field_bytes=16,
            )["data_rows"], 1)


if __name__ == "__main__":
    unittest.main()

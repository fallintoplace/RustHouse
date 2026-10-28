import tempfile
import unittest
from pathlib import Path

from bench.scale.csv_stream_to_tsv import convert, escape_field


class CsvToTsvTests(unittest.TestCase):
    def test_quotes_embedded_newlines_tabs_and_backslashes(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_text('id,value\n1,"a,b"\n2,"line\nnext"\n3,"a\tb"\n4,"\\N"\n')
            stats = convert(source, target, header=True)
            self.assertEqual(stats["data_rows"], 4)
            self.assertEqual(target.read_bytes(),
                             b"id\tvalue\n1\ta,b\n2\tline\\nnext\n3\ta\\tb\n4\t\\\\N\n")

    def test_explicit_null_mode_and_empty_strings(self):
        self.assertEqual(escape_field("", False), "")
        self.assertEqual(escape_field("", True), r"\N")
        self.assertEqual(escape_field("\x00\r\v"), r"\0\r\v")

    def test_rejects_column_mismatch_and_same_file(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_text("x,y\na\n")
            with self.assertRaisesRegex(ValueError, "column count"):
                convert(source, target)
            with self.assertRaises(ValueError):
                convert(source, source)


if __name__ == "__main__":
    unittest.main()

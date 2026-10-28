import tempfile
import unittest
from pathlib import Path

from bench.scale.csv_stream_to_tsv import convert, escape_field


class CsvToTsvTests(unittest.TestCase):
    def test_quotes_embedded_newlines_tabs_and_backslashes(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_bytes('id,value\n1,"a,b"\n2,"line\nnext"\n3,"a\tb"\n4,"\\N"\n'.encode("utf-8"))
            stats = convert(source, target, header=True)
            self.assertEqual(stats["data_rows"], 4)
            self.assertEqual(target.read_bytes(),
                             b"id\tvalue\n1\ta,b\n2\tline\\nnext\n3\ta\\tb\n4\t\\\\N\n")

    def test_preserves_real_crlf_inside_quoted_fields(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_bytes(b'id,value\r\n1,"first\r\nsecond"\r\n')
            stats = convert(source, target, header=True)
            self.assertEqual(stats["data_rows"], 1)
            self.assertEqual(target.read_bytes(), b"id\tvalue\n1\tfirst\\r\\nsecond\n")

    def test_explicit_null_mode_and_empty_strings(self):
        self.assertEqual(escape_field("", False), "")
        self.assertEqual(escape_field("", True), r"\N")
        self.assertEqual(escape_field("\x00\r\v"), r"\0\r\v")

    def test_escape_translation_matches_character_reference(self):
        substitutions = {"\\": r"\\", "\t": r"\t", "\n": r"\n",
                         "\r": r"\r", "\x00": r"\0", "\b": r"\b",
                         "\f": r"\f", "\v": r"\v"}
        for value in ["", "plain 123", "a\\b\tc\né", "\0\b\f\v\r",
                      "x" * 65536 + "\n" + "y" * 65536]:
            expected = "".join(substitutions.get(char, char) for char in value)
            self.assertEqual(escape_field(value), expected)

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

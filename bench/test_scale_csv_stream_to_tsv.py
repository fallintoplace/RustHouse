import tempfile
import unittest
from pathlib import Path

from bench.scale.csv_stream_to_tsv import convert, escape_field, utf8_length


class CsvToTsvTests(unittest.TestCase):
    def test_refuses_symlinked_csv_source_before_writing(self):
        from bench.scale.csv_stream_to_tsv import convert
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            source = directory / "source.csv"
            source.write_text("x\n")
            link = directory / "alias.csv"
            try:
                link.symlink_to(source)
            except (OSError, NotImplementedError):
                self.skipTest("symlink creation unavailable")
            target = directory / "out.tsv"
            with self.assertRaisesRegex(ValueError, "symlink"):
                convert(link, target)
            self.assertFalse(target.exists())


    def test_does_not_unlink_a_preexisting_pending_file(self):
        import os
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            source = folder / "input.csv"
            output = folder / "out.tsv"
            source.write_bytes(b"1,hello\n")
            old_pending = folder / f".out.tsv.{os.getpid()}.pending"
            old_pending.write_bytes(b"must survive")
            convert(source, output)
            self.assertEqual(output.read_text(), "1\thello\n")
            self.assertEqual(old_pending.read_bytes(), b"must survive")

    def test_random_pending_name_collision_is_not_deleted(self):
        from unittest.mock import patch
        import os
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            source = folder / "input.csv"
            output = folder / "out.tsv"
            source.write_bytes(b"1,hello\n")
            collision = folder / f".out.tsv.{os.getpid()}.collision.pending"
            collision.write_bytes(b"existing")
            with patch("bench.scale.csv_stream_to_tsv.secrets.token_hex", return_value="collision"):
                with self.assertRaises(FileExistsError):
                    convert(source, output)
            self.assertEqual(collision.read_bytes(), b"existing")
            self.assertFalse(output.exists())

    def test_quotes_embedded_newlines_tabs_and_backslashes(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_bytes('id,value\n1,"a,b"\n2,"line\nnext"\n3,"a\tb"\n4,"\\N"\n'.encode("utf-8"))
            stats = convert(source, target, header=True)
            self.assertEqual(stats["data_rows"], 4)
            self.assertEqual(target.read_bytes(),
                             b"id\tvalue\n1\ta,b\n2\tline\\nnext\n3\ta\\tb\n4\t\\\\N\n")

    def test_utf8_sizes_and_unicode_field_budget(self):
        self.assertEqual(utf8_length("1" * 100000), 100000)
        self.assertEqual(utf8_length("é🙂"), 6)
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "utf8.csv"
            target = Path(temp) / "utf8.tsv"
            source.write_text("é,🙂\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "oversized"):
                convert(source, target, max_field_bytes=3)
            self.assertFalse(target.exists())
            report = convert(source, target, max_field_bytes=4)
            self.assertEqual(report["data_rows"], 1)
            self.assertEqual(target.read_bytes(), "é\t🙂\n".encode("utf-8"))

    def test_preserves_real_crlf_inside_quoted_fields(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "in.csv"
            target = Path(temp) / "out.tsv"
            source.write_bytes(b'id,value\r\n1,"first\r\nsecond"\r\n')
            stats = convert(source, target, header=True)
            self.assertEqual(stats["data_rows"], 1)
            self.assertEqual(target.read_bytes(), b"id\tvalue\n1\tfirst\\r\\nsecond\n")

    def test_published_tsv_fsyncs_directory(self):
        import os
        from unittest.mock import patch
        if os.name != "posix":
            self.skipTest("directory fsync requires POSIX")
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "in.csv"
            target = Path(folder) / "out.tsv"
            source.write_text("1,hello\n")
            with patch("bench.scale.csv_stream_to_tsv.os.fsync", wraps=os.fsync) as fsync:
                convert(source, target)
            self.assertGreaterEqual(fsync.call_count, 2)
            self.assertEqual(target.read_text(), "1\thello\n")

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

    def test_can_remove_csv_header_for_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "input.csv"
            target = Path(temp) / "copy.tsv"
            source.write_text("id,name\n1,alpha\n2,beta\n")
            report = convert(source, target, header=True, skip_header=True)
            self.assertEqual(report["records"], 3)
            self.assertEqual(report["data_rows"], 2)
            self.assertEqual(target.read_bytes(), b"1\talpha\n2\tbeta\n")
        with self.assertRaisesRegex(ValueError, "requires"):
            convert("input.csv", "output.tsv", skip_header=True)


class CSVFieldBudgetTypeTests(unittest.TestCase):
    def test_rejects_invalid_field_size_before_opening_files(self):
        for limit in (True, 1.5, float("nan"), float("inf"), "64"):
            with self.subTest(limit=limit):
                with self.assertRaisesRegex(ValueError, "positive integer"):
                    convert("not-present.csv", "out.tsv", max_field_bytes=limit)

    def test_optional_expected_column_count_rejects_mismatched_exports(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "input.csv"
            output = Path(root) / "copy.tsv"
            source.write_bytes(b"id,value\n1,a\n")
            with self.assertRaisesRegex(ValueError, "expected_columns"):
                convert(source, output, header=True, expected_columns=3)
            self.assertFalse(output.exists())
            report = convert(source, output, header=True, expected_columns=2)
            self.assertEqual(report["columns"], 2)
            for invalid in (0, 129, True, "2"):
                with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, "expected_columns"):
                    convert("missing.csv", "output.tsv", expected_columns=invalid)

if __name__ == "__main__":
    unittest.main()

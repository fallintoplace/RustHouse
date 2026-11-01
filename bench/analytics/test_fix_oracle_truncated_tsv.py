"""Reject partial final TSV rows when checking SQLite reference results."""
import unittest

from sqlite_oracle import assert_matches, parse_rows


class OracleTrailingNewlineTests(unittest.TestCase):
    def test_nonempty_output_needs_complete_last_row(self):
        with self.assertRaisesRegex(ValueError, "final newline"):
            parse_rows(b"42")
        with self.assertRaisesRegex(ValueError, "final newline"):
            assert_matches(b"42", [(42,)])
        self.assertEqual(parse_rows(b"42\n"), [["42"]])
        self.assertEqual(parse_rows(b""), [])

    def test_valid_crlf_and_escaped_newline_are_preserved(self):
        self.assertEqual(parse_rows(b"a\\nb\r\n"), [[r"a\nb"]])
        assert_matches(b"42\r\n", [(42,)])


if __name__ == "__main__":
    unittest.main()

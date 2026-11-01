"""Keep the benchmark format classification aligned with the Rust SQL lexer."""
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.common import _leading_sql_keyword, cli_sql


class NestedLeadingCommentsTests(unittest.TestCase):
    def test_leading_sql_word_recognizes_nested_and_adjacent_comments(self):
        for statement in (
            "/* outer /* nested */ end */ SELECT 1",
            "/* comment */SELECT/* comment */ 1",
            "-- first\r# second\n/* note */\vSELECT 1",
            "\ufeff/* header */SELECT 1",
        ):
            with self.subTest(statement=statement):
                self.assertEqual(_leading_sql_keyword(statement), "SELECT")
        self.assertEqual(_leading_sql_keyword("SELECTED 1"), "SELECTED")

    def test_malformed_comments_fail_closed(self):
        for statement in ("/* never closed SELECT 1", "/*" * 33 + "*/" * 33 + "SELECT 1"):
            self.assertEqual(_leading_sql_keyword(statement), "")

    @patch("bench.common.subprocess.run")
    def test_select_format_survives_nested_comments(self, run):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b'{"n":1}\n')
            return SimpleNamespace(returncode=0)
        run.side_effect = execute
        result = cli_sql("rusthouse", "db", "/* a /* b */ c */SELECT 1", 5,
                         select_format="jsonl")
        self.assertEqual(result, b'{"n":1}\n')
        self.assertEqual(run.call_args.args[0][-2:], ["--format", "jsonl"])

    @patch("bench.common.subprocess.run")
    def test_unsafe_statement_does_not_inherit_select_format(self, run):
        with self.assertRaises(ValueError):
            cli_sql("rusthouse", "db", "/* a /* b */ c */INSERT INTO t VALUES(1)",
                    5, select_format="jsonl")
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()

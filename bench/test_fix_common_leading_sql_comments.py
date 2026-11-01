import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.common import cli_sql


class CommentedSqlFormatTests(unittest.TestCase):
    @patch("bench.common.subprocess.run")
    def test_select_after_comments_uses_requested_json_verification_format(self, run):
        commands = []
        def invoke(command, **kwargs):
            commands.append(command)
            kwargs["stdout"].write(b'{"value":1}\n')
            return SimpleNamespace(returncode=0)
        run.side_effect = invoke
        for statement in ("-- a note\n SELECT 1", "/* comment */ SELECT 1",
                          "# a note\n SELECT 1"):
            with self.subTest(statement=statement):
                output = cli_sql("binary", "db", statement, 5, select_format="jsonl")
                self.assertEqual(output, b'{"value":1}\n')
                self.assertEqual(commands[-1][-2:], ["--format", "jsonl"])

    @patch("bench.common.subprocess.run")
    def test_comment_before_mutation_does_not_enable_select_format(self, run):
        with self.assertRaisesRegex(ValueError, "only with SELECT"):
            cli_sql("binary", "db", "/* note */ INSERT INTO t VALUES (1)", 5,
                    select_format="jsonl")
        run.assert_not_called()


    @patch("bench.common.subprocess.run")
    def test_nested_comments_and_adjacent_tokens_keep_select_format(self, run):
        def invoke(command, **kwargs):
            kwargs["stdout"].write(b'{"value":1}\n')
            return SimpleNamespace(returncode=0)
        run.side_effect = invoke
        for statement in (
                "/* outer /* inner */ tail */SELECT 1",
                "/* outer */ /* second */ SELECT(1)",
                "-- first\n# second\nSELECT 1",
                "SELECT/* adjacent */ 1",
                "/* outer /* nested */ tail */\nSELECT 1"):
            with self.subTest(statement=statement):
                output = cli_sql("binary", "db", statement, 5, select_format="jsonl")
                self.assertEqual(output, b'{"value":1}\n')
                self.assertEqual(run.call_args.args[0][-2:], ["--format", "jsonl"])

    @patch("bench.common.subprocess.run")
    def test_nested_comments_cannot_hide_mutation(self, run):
        for statement in (
                "/* outer /* inner */ tail */ INSERT INTO t VALUES (1)",
                "/* comment */DELETE FROM t",
                "/* unfinished SELECT 1"):
            with self.subTest(statement=statement):
                with self.assertRaisesRegex(ValueError, "only with SELECT"):
                    cli_sql("binary", "db", statement, 5, select_format="jsonl")
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()

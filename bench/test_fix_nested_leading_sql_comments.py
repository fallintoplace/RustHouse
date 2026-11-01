import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.common import _leading_sql_keyword, cli_sql


class NestedLeadingSqlCommentsTests(unittest.TestCase):
    def test_nested_comments_are_skipped_before_classifying_sql(self):
        for sql in (
            "/* outer /* inner */ outer */ SELECT 1",
            "/* a /* b /* c */ b */ a */ -- note\nSELECT 1",
            "# leading\n/* a /* b */ a */ SELECT 1",
        ):
            self.assertEqual(_leading_sql_keyword(sql), "SELECT", sql)

    def test_incomplete_outer_block_comment_is_not_a_select(self):
        for sql in (
            "/* outer /* inner */ SELECT 1",
            "/* a /* b /* c */ b */ SELECT 1",
        ):
            self.assertEqual(_leading_sql_keyword(sql), "", sql)

    @patch("bench.common.subprocess.run")
    def test_cli_select_format_survives_nested_comments(self, run):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"1\n")
            return SimpleNamespace(returncode=0)

        run.side_effect = execute
        sql = "/* outer /* inner */ outer */ SELECT 1"
        self.assertEqual(cli_sql("rusthouse", "db", sql, 5), b"1\n")
        self.assertEqual(run.call_args.args[0][-2:], ["--format", "tsv-no-header"])


if __name__ == "__main__":
    unittest.main()

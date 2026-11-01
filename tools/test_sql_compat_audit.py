import unittest
from unittest.mock import Mock
from pathlib import Path
import tempfile
from sql_compat_audit import audit, parse_lines, read_queries


class AuditTests(unittest.TestCase):
    def test_hash_comments_do_not_create_executable_separators(self):
        from sql_compat_audit import _has_separator, parse_lines
        self.assertFalse(_has_separator("SELECT 1 # comment with ; semicolons"))
        self.assertTrue(_has_separator("SELECT 1; # real separator"))
        self.assertEqual(
            parse_lines("# ignored line\nSELECT 1 # comment with ;\nSELECT 2;"),
            [(2, "SELECT 1 # comment with"), (3, "SELECT 2")],
        )

    def test_accepts_one_select_per_line(self):
        self.assertEqual(parse_lines("-- note\nSELECT 1;\nSELECT count(*) FROM numbers(3)\n"),
                         [(2, "SELECT 1"), (3, "SELECT count(*) FROM numbers(3)")])

    def test_audit_corpus_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "queries.sql"
            source.write_text("SELECT 1;")
            alias = Path(directory) / "alias.sql"
            alias.symlink_to(source)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                read_queries(alias)
            self.assertEqual(read_queries(source), [(1, "SELECT 1")])

    def test_rejects_corpus_replaced_between_preflight_and_open(self):
        import os
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus = root / "queries.sql"
            corpus.write_text("SELECT 1\n")
            replacement = root / "replacement.sql"
            replacement.write_text("SELECT 2\n")
            real_open = os.open

            def race(path, flags):
                corpus.unlink()
                replacement.rename(corpus)
                return real_open(path, flags)

            with patch("sql_compat_audit.os.open", side_effect=race):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    read_queries(corpus)

    def test_audit_file_fails_early_on_large_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "queries.sql"
            path.write_bytes(b"-" * (1_048_576 + 1))
            with self.assertRaisesRegex(ValueError, "1 MiB"):
                read_queries(path)
            path.write_text("-- note\nSELECT 2;\n", encoding="utf-8")
            self.assertEqual(read_queries(path), [(2, "SELECT 2")])

    def test_accepts_semicolons_inside_sql_literals_only(self):
        self.assertEqual(parse_lines("SELECT ';' AS punctuation;"), [
            (1, "SELECT ';' AS punctuation")])
        self.assertEqual(parse_lines('SELECT "column;id" FROM table;'), [
            (1, 'SELECT "column;id" FROM table')])
        self.assertEqual(parse_lines("SELECT 'it''s;a;literal';"), [
            (1, "SELECT 'it''s;a;literal'")])
        for sql in ("SELECT 1; DELETE FROM t;", "SELECT 'unterminated",
                    "SELECT 1; SELECT 2"):
            with self.subTest(sql=sql), self.assertRaisesRegex(ValueError, "one bounded SELECT"):
                parse_lines(sql)

    def test_ignores_statement_separators_inside_line_comments(self):
        self.assertEqual(parse_lines("SELECT 1 -- comment; not SQL"), [
            (1, "SELECT 1 -- comment; not SQL")])
        with self.assertRaisesRegex(ValueError, "one bounded SELECT"):
            parse_lines("SELECT 1; -- second statement")

    def test_block_comments_do_not_create_fake_statement_separators(self):
        self.assertEqual(
            parse_lines("SELECT 1 /* quote ' and ; are comments */;"),
            [(1, "SELECT 1 /* quote ' and ; are comments */")],
        )
        for query in ("SELECT 1 /* unterminated ;",
                      "SELECT 1 /* comment */; SELECT 2"):
            with self.subTest(query=query), self.assertRaisesRegex(
                ValueError, "one bounded SELECT"
            ):
                parse_lines(query)

    def test_rejects_mutating_sql(self):
        with self.assertRaises(ValueError):
            parse_lines("DROP TABLE events;")
        with self.assertRaises(ValueError):
            parse_lines("SELECT 1; DROP TABLE events;")

    def test_fail_fast_stops_after_first_parser_error(self):
        run = Mock()
        run.return_value.returncode = 1
        run.return_value.stderr = "unsupported"
        cases = [(1, "SELECT bad"), (2, "SELECT ok"), (3, "SELECT later")]
        report = audit("rusthouse", cases, run, fail_fast=True)
        self.assertEqual(report["total"], 1)
        self.assertEqual(report["requested"], 3)
        self.assertTrue(report["stopped_early"])
        self.assertEqual(run.call_count, 1)
        run.reset_mock()
        complete = audit("rusthouse", cases, run)
        self.assertEqual(complete["total"], 3)
        self.assertFalse(complete["stopped_early"])

    def test_large_cli_output_is_spooled_and_failure_reason_is_bounded(self):
        def emit(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            self.assertNotIn("text", kwargs)
            kwargs["stdout"].write(b"x" * 100_000)
            kwargs["stderr"].write(b"y" * 100_000)
            return type("Result", (), {"returncode": 1})()
        result = audit("rusthouse", [(1, "SELECT 1")], emit)
        self.assertFalse(result["results"][0]["parser_accepted"])
        self.assertEqual(len(result["results"][0]["reason"]), 500)

    def test_explain_only_and_counts_rejections(self):
        run = Mock()
        run.return_value.returncode = 1
        run.return_value.stderr = "unsupported SQL"
        report = audit("rusthouse", [(1, "SELECT bad")], run)
        self.assertEqual(report["accepted"], 0)
        self.assertIn("--explain", run.call_args.args[0])
        self.assertIn("--database", run.call_args.args[0])


    def test_custom_per_query_timeout_and_invalid_values(self):
        run = Mock()
        run.return_value.returncode = 0
        run.return_value.stderr = ""
        report = audit("rusthouse", [(1, "SELECT 1")], run, timeout=0.5)
        self.assertEqual(report["accepted"], 1)
        self.assertEqual(run.call_args.kwargs["timeout"], 0.5)
        for invalid in (0, -1, float("inf"), float("nan"), True):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, "timeout"):
                audit("rusthouse", [(1, "SELECT 1")], run, timeout=invalid)

if __name__ == "__main__":
    unittest.main()

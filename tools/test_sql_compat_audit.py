import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock
from sql_compat_audit import audit, parse_lines, read_queries_file


class AuditTests(unittest.TestCase):
    def test_accepts_one_select_per_line(self):
        self.assertEqual(parse_lines("-- note\nSELECT 1;\nSELECT count(*) FROM numbers(3)\n"),
                         [(2, "SELECT 1"), (3, "SELECT count(*) FROM numbers(3)")])

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

    def test_explain_only_and_counts_rejections(self):
        run = Mock()
        run.return_value.returncode = 1
        run.return_value.stderr = "unsupported SQL"
        report = audit("rusthouse", [(1, "SELECT bad")], run)
        self.assertEqual(report["accepted"], 0)
        self.assertIn("--explain", run.call_args.args[0])
        self.assertIn("--database", run.call_args.args[0])

    def test_input_file_is_bounded_before_parsing(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "queries.sql"
            source.write_bytes(b"SELECT 1\n")
            self.assertEqual(read_queries_file(source), [(1, "SELECT 1")])
            source.write_bytes(b" " * 1_048_577)
            with self.assertRaisesRegex(ValueError, "1 MiB"):
                read_queries_file(source)
            source.write_bytes(b"SELECT \xff")
            with self.assertRaises(UnicodeError):
                read_queries_file(source)


if __name__ == "__main__":
    unittest.main()

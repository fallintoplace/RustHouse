import unittest
from unittest.mock import Mock
from sql_compat_audit import audit, parse_lines


class AuditTests(unittest.TestCase):
    def test_accepts_one_select_per_line(self):
        self.assertEqual(parse_lines("-- note\nSELECT 1;\nSELECT count(*) FROM numbers(3)\n"),
                         [(2, "SELECT 1"), (3, "SELECT count(*) FROM numbers(3)")])

    def test_rejects_mutating_sql(self):
        with self.assertRaises(ValueError):
            parse_lines("DROP TABLE events;")
        with self.assertRaises(ValueError):
            parse_lines("SELECT 1; DROP TABLE events;")

    def test_explain_only_and_counts_rejections(self):
        run = Mock()
        run.return_value.returncode = 1
        run.return_value.stderr = "unsupported SQL"
        report = audit("rusthouse", [(1, "SELECT bad")], run)
        self.assertEqual(report["accepted"], 0)
        self.assertIn("--explain", run.call_args.args[0])
        self.assertIn("--database", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()

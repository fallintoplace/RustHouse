"""Resource profiles must accept only one read-only SQL statement."""
import unittest

from bench.scale.query_resource_profile import validate_select


class ResourceProfilerSqlTests(unittest.TestCase):
    def test_semicolons_in_sql_literals_or_comments(self):
        for sql in ("SELECT ';' AS text",
                    "SELECT 'a; b' AS text;",
                    "SELECT 1 /* ; ; */",
                    "SELECT 1 -- ; ignored\n"):
            with self.subTest(sql=sql):
                validate_select(sql)

    def test_rejects_extra_statements_and_unterminated_syntax(self):
        for sql in ("SELECT 1; DROP TABLE events", "DELETE FROM events",
                    "SELECT 'unterminated", "SELECT 1 /* unclosed"):
            with self.subTest(sql=sql), self.assertRaises(ValueError):
                validate_select(sql)


if __name__ == "__main__":
    unittest.main()

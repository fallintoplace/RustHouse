import unittest

from bench.scale.query_digest import query_digest


class QueryDigestInputValidationTests(unittest.TestCase):
    def test_rejects_boolean_or_fractional_max_rows_before_execution(self):
        for invalid in (True, False, 1.5, "100", 0):
            with self.subTest(rows=invalid), self.assertRaisesRegex(
                    ValueError, "max_rows"):
                query_digest("unused", "unused", "SELECT 1", max_rows=invalid,
                             run=lambda *a, **k: self.fail("should not execute"))

    def test_rejects_empty_nonstring_or_oversized_sql_before_execution(self):
        for sql in ("", "  ", 123, "SELECT '" + "x" * 65_536 + "'"):
            with self.subTest(length=len(str(sql))), self.assertRaisesRegex(
                    ValueError, "SQL must"):
                query_digest("unused", "unused", sql,
                             run=lambda *a, **k: self.fail("should not execute"))


if __name__ == "__main__":
    unittest.main()

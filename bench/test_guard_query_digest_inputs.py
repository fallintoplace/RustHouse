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


    def test_accepts_commented_select_without_weakening_read_only_gate(self):
        from types import SimpleNamespace

        def run(command, **kwargs):
            kwargs["stdout"].write(b"1\n")
            return SimpleNamespace(returncode=0)

        for statement in ("/* note */ SELECT 1", "-- note\nSELECT 1",
                          "# note\nSELECT 1"):
            with self.subTest(statement=statement):
                report = query_digest("rusthouse", "part", statement, run=run)
                self.assertEqual(report["bytes"], 2)
                self.assertEqual(report["sha256"],
                                 __import__("hashlib").sha256(b"1\n").hexdigest())

        for statement in ("/* note */ DELETE FROM t",
                          "-- note\nINSERT INTO t VALUES (1)",
                          "/* SELECT 1 */ SELECT 1; DELETE FROM t"):
            with self.subTest(statement=statement), self.assertRaisesRegex(
                    ValueError, "read-only SELECT"):
                query_digest("rusthouse", "part", statement,
                             run=lambda *a, **k: self.fail("must not run"))

if __name__ == "__main__":
    unittest.main()

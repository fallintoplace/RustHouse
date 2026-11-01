import hashlib
import unittest
from types import SimpleNamespace

from bench.scale.query_digest import query_digest


class QueryDigestCommentPrefixTests(unittest.TestCase):
    @staticmethod
    def run(command, *, stdout, stderr, **kwargs):
        stdout.write(b"1\n")
        return SimpleNamespace(returncode=0)

    def test_accepts_sql_comments_before_read_only_select(self):
        for sql in (
            "-- run notes\nSELECT 1",
            "# run notes\nSELECT 1",
            "/* outer /* inner */ closed */ SELECT 1",
            "  /* note */ -- line\n SELECT 1",
        ):
            report = query_digest("rusthouse", "part", sql, run=self.run,
                                  expected_bytes=2,
                                  expected_sha256=hashlib.sha256(b"1\n").hexdigest())
            self.assertTrue(report["verified"], sql)

    def test_rejects_non_select_and_multi_statement_inputs(self):
        for sql in (
            "-- note\nDELETE FROM t",
            "/* unclosed SELECT 1",
            "/* " * 33 + " */" * 33 + "SELECT 1",
            "-- notes only",
            "/* note */ SELECT 1; SELECT 2",
        ):
            with self.assertRaisesRegex(ValueError, "read-only SELECT"):
                query_digest("rusthouse", "part", sql, run=self.run)


if __name__ == "__main__":
    unittest.main()

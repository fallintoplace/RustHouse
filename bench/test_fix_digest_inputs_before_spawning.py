import unittest
from bench.scale.query_digest import query_digest

class DigestArgumentsTests(unittest.TestCase):
    def test_rejects_untyped_limits_before_query(self):
        for rows in (True, 1.5, 0):
            with self.subTest(rows=rows), self.assertRaisesRegex(ValueError, "max_rows"):
                query_digest("rusthouse", ".", "SELECT 1", max_rows=rows,
                             run=lambda *args, **kwargs: self.fail("query started"))

    def test_rejects_nonstring_hash(self):
        with self.assertRaisesRegex(ValueError, "expected_sha256"):
            query_digest("rusthouse", ".", "SELECT 1", expected_sha256=123,
                         run=lambda *args, **kwargs: self.fail("query started"))

if __name__ == "__main__":
    unittest.main()

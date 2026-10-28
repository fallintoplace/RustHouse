import math
import unittest

from bench.scale.topk_probe import probe


class TopKInputsTests(unittest.TestCase):
    def test_rejects_fractional_and_boolean_row_domains(self):
        for rows in (True, 3.5):
            with self.subTest(rows=rows), self.assertRaisesRegex(ValueError, "rows"):
                probe("rusthouse", ".", rows, execute=lambda *a, **kw: self.fail("query executed"))

    def test_rejects_fractional_and_boolean_limits(self):
        for limit in (True, 1.5):
            with self.subTest(limit=limit), self.assertRaisesRegex(ValueError, "limit"):
                probe("rusthouse", ".", 8, limit=limit,
                      execute=lambda *a, **kw: self.fail("query executed"))

    def test_rejects_invalid_timeouts_before_query(self):
        for timeout in (math.nan, math.inf, -math.inf, True, 0, -1):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                probe("rusthouse", ".", 8, timeout=timeout,
                      execute=lambda *a, **kw: self.fail("query executed"))


if __name__ == "__main__":
    unittest.main()

import math
import unittest

from bench.scale.query_digest import query_digest


class DigestInputTests(unittest.TestCase):
    def test_rejects_invalid_timeout_before_spawning(self):
        for timeout in (0, -1, math.nan, math.inf, -math.inf, True):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "positive and finite"):
                query_digest("rusthouse", ".", "SELECT 1", timeout=timeout,
                             run=lambda *args, **kw: self.fail("unexpected query"))

    def test_rejects_fractional_or_boolean_byte_limits(self):
        for limit in (0, -1, 1.5, True):
            with self.subTest(limit=limit), self.assertRaisesRegex(ValueError, "positive and finite"):
                query_digest("rusthouse", ".", "SELECT 1", max_output_bytes=limit,
                             run=lambda *args, **kw: self.fail("unexpected query"))


if __name__ == "__main__":
    unittest.main()

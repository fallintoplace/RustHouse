import math
import unittest
from bench.scale.query_correctness_soak import percentile, soak

class SoakInputTests(unittest.TestCase):
    def test_invalid_percentile_types(self):
        for value in (True, "90", math.inf, math.nan, -1, 101):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "percentile"):
                percentile([1, 2, 3], value)

    def test_invalid_timeouts_fail_before_reference_read(self):
        for value in (True, "30", math.nan, math.inf, 0):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "timeout"):
                soak("rusthouse", "db", "SELECT 1", "/nonexistent.tsv", timeout=value)

if __name__ == "__main__":
    unittest.main()

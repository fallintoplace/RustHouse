import math
import unittest
from bench.compare import compare

class ComparisonThresholdTests(unittest.TestCase):
    def test_rejects_invalid_threshold_before_report_processing(self):
        for value in (True, "10", math.nan, math.inf, -1):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "slowdown threshold"):
                compare({}, {}, slowdown_threshold_percent=value)

if __name__ == "__main__":
    unittest.main()

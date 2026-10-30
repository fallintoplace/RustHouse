import math
import unittest
from bench.scale.physical_gate import validate

class GateTargetTypesTests(unittest.TestCase):
    def test_invalid_row_floors(self):
        for value in (True, 1.5, 0):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "min_rows"):
                validate({}, min_rows=value)

    def test_invalid_timing_targets(self):
        for value in (True, "5", math.nan, math.inf, 0):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "timing targets"):
                validate({}, max_median_seconds=value)
                validate({}, max_p90_seconds=value)

if __name__ == "__main__":
    unittest.main()

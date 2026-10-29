import math
import unittest
from bench.scale.compare_physical import compare

class ComparisonInputsTests(unittest.TestCase):
    def test_rejects_nonobject_reports(self):
        for left, right in (([], {}), ({}, []), (None, {})):
            with self.subTest(left=left, right=right), self.assertRaisesRegex(ValueError, "objects"):
                compare(left, right)

    def test_rejects_invalid_budgets(self):
        for budget in (True, "10", math.nan, math.inf, -1):
            with self.subTest(budget=budget), self.assertRaisesRegex(ValueError, "max_slowdown_percent"):
                compare({}, {}, max_slowdown_percent=budget)

if __name__ == "__main__":
    unittest.main()

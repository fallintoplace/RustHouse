import unittest
from bench.pr_parts_compare import CASES, FIELDS, compare_text

def benchmark(median):
    return ",".join(FIELDS) + "\n" + "".join(
        f"{name},{median},{median},1.0\n" for name in CASES
    )

class PersistedPartsComparisonOverflowTests(unittest.TestCase):
    def test_extreme_finite_medians_do_not_generate_infinite_change(self):
        summary = compare_text(benchmark("1e-300"), benchmark("1e300"))
        self.assertEqual(summary.count("N/A (ratio overflow)"), len(CASES))
        self.assertNotIn("inf%", summary)

    def test_ordinary_ratios_remain_percentages(self):
        summary = compare_text(benchmark("2.0"), benchmark("1.0"))
        self.assertIn("-50.0%", summary)

if __name__ == "__main__":
    unittest.main()

import unittest
from bench.pr_grouped_compare import CASES, compare_text

def benchmark(median):
    return ("RustHouse grouped scans: 100 rows, 10 keys, 5 samples\n"
            + "".join(f"{name} {median} {median} 1.0\n" for name in CASES))

class GroupedRatioOverflowTests(unittest.TestCase):
    def test_extreme_finite_medians_use_unavailable_marker(self):
        tiny = "0." + "0" * 299 + "1"
        huge = "1" + "0" * 300 + ".0"
        summary = compare_text(benchmark(tiny), benchmark(huge))
        self.assertEqual(summary.count("N/A (ratio overflow)"), len(CASES))
        self.assertNotIn("inf%", summary)

    def test_regular_comparison_still_reports_percentage(self):
        summary = compare_text(benchmark("2.000"), benchmark("1.000"))
        self.assertIn("-50.0%", summary)

if __name__ == "__main__":
    unittest.main()

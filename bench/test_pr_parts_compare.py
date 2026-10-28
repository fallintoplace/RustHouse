import unittest

from bench.pr_parts_compare import CASES, compare_text, parse_benchmark


def fixture(milliseconds=1.0):
    lines = ["workload,median_ms,p90_ms,million_logical_rows_per_second"]
    for name in CASES:
        lines.append(f"{name},{milliseconds:.6f},{2 * milliseconds:.6f},1.000000")
    return "\n".join(lines) + "\n"


class PersistedPartComparisonTests(unittest.TestCase):
    def test_parses_verified_workloads_and_renders_comparison(self):
        a = parse_benchmark(fixture(2))
        self.assertEqual(set(a), set(CASES))
        rendered = compare_text(fixture(2), fixture(1))
        self.assertIn("count 100%", rendered)
        self.assertIn("-50.0%", rendered)

    def test_requires_every_case_once(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            parse_benchmark(fixture().replace("filtered top-K,1.000000,2.000000,1.000000\n", ""))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            parse_benchmark(fixture() + "count 1%,1.000000,2.000000,1.000000\n")

    def test_rejects_unusable_measurements(self):
        with self.assertRaisesRegex(ValueError, "non-positive"):
            parse_benchmark(fixture(0))
        with self.assertRaisesRegex(ValueError, "p90 below median"):
            parse_benchmark(fixture().replace("1.000000,2.000000", "2.000000,1.000000"))
        with self.assertRaisesRegex(ValueError, "invalid timing"):
            parse_benchmark(fixture().replace("1.000000,2.000000", "oops,2.000000"))


if __name__ == "__main__":
    unittest.main()

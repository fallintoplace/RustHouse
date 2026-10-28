import unittest

from bench.pr_grouped_compare import compare_text, parse_benchmark


def output(rows=65536, keys=413, samples=11, min_ms=4.0):
    return (
        f"RustHouse grouped scans: {rows} rows, {keys} keys, {samples} samples\n"
        "aggregate     median ms       p90 ms         M rows/s\n"
        f"min             {min_ms:.3f}          5.000             16.38\n"
        "avg             5.000          6.000             13.11\n"
        "max             6.000          7.000             10.92\n"
    )


class GroupedBenchmarkComparisonTests(unittest.TestCase):
    def test_detects_faster_candidate(self):
        report = compare_text(output(), output(min_ms=2.0))
        self.assertIn("GROUP BY performance", report)
        self.assertIn("-50.0%", report)

    def test_rejects_changed_fixture(self):
        with self.assertRaisesRegex(ValueError, "settings differ"):
            compare_text(output(rows=65536), output(rows=32768))

    def test_rejects_duplicate_or_missing_cases(self):
        broken = output() + "min 4.000 5.000 16.38\n"
        with self.assertRaisesRegex(ValueError, "duplicate"):
            parse_benchmark(broken)
        with self.assertRaisesRegex(ValueError, "missing grouped"):
            parse_benchmark(output().replace("max             6.000          7.000             10.92\n", ""))

    def test_rejects_zero_and_incompatible_samples(self):
        with self.assertRaisesRegex(ValueError, "positive"):
            parse_benchmark(output(min_ms=0.0))
        with self.assertRaisesRegex(ValueError, "settings differ"):
            compare_text(output(samples=11), output(samples=9))


if __name__ == "__main__":
    unittest.main()

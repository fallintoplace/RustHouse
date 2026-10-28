"""Physical hash JOIN benchmark comparison regressions."""

import unittest

from bench.pr_join_compare import compare_text, parse_benchmark


def sample(median="2.000000", p90="2.500000", rows=8192, build=4096, samples=11):
    return (
        f"RustHouse hash JOIN benchmark: {rows} probe rows, {build} build rows, "
        f"{samples} samples, 3 warmups\n"
        "workload median_ms p90_ms million_probe_rows_per_second\n"
        f"inner equi-join {median} {p90} 5.0000\n"
    )


class JoinCompareTests(unittest.TestCase):
    def test_reports_matching_verified_fixtures(self):
        result = compare_text(sample(), sample("1.500000", "2.000000"))
        self.assertIn("Median (ms) | 2.000000 | 1.500000", result)
        self.assertIn("-25.0%", result)
        self.assertIn("1.333x", result)
        self.assertIn("not a ClickHouse comparison", result)

    def test_rejects_different_rows_and_sample_counts(self):
        for changed in [sample(rows=4096, build=2048), sample(samples=7)]:
            with self.assertRaisesRegex(ValueError, "fixtures differ"):
                compare_text(sample(), changed)

    def test_rejects_missing_duplicate_and_invalid_samples(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            parse_benchmark(sample().replace("inner equi-join", "ignored"))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            parse_benchmark(sample() + "inner equi-join 2.000000 2.500000 5.0000\n")
        with self.assertRaisesRegex(ValueError, "p90"):
            parse_benchmark(sample("2.000000", "1.000000"))
        with self.assertRaisesRegex(ValueError, "positive"):
            parse_benchmark(sample("0.000000", "1.000000"))


if __name__ == "__main__":
    unittest.main()

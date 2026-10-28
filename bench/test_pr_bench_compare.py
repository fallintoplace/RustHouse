"""Tests for the base-versus-PR benchmark summary."""

import unittest
from bench.pr_bench_compare import compare_text, parse_benchmark


def report(rows=200000, topk=12.500, samples=21):
    return (
        f"RustHouse numbers() benchmarks: {rows} input rows, {samples} samples, 3 warmups\n"
        "workload median ms M input rows/s\n"
        "filtered count 0.000 0.00\n"
        "filtered sum 0.003 1.00\n"
        f"filtered top-K {topk:.3f} 2.00\n"
    )


class CompareRustBenchTests(unittest.TestCase):
    def test_reports_improvements_and_sub_microsecond_measurements(self):
        result = compare_text(report(), report(topk=1.250))
        self.assertIn("| filtered top-K | 12.500000 | 1.250000 | -90.0% |", result)
        self.assertIn("N/A (below printed precision)", result)
        self.assertIn("informational, not a required CI gate", result)

    def test_submicrosecond_improvements_are_not_rounded_to_zero(self):
        baseline = report().replace("filtered count 0.000 0.00", "filtered count 0.000123 0.00")
        candidate = report().replace("filtered count 0.000 0.00", "filtered count 0.000082 0.00")
        summary = compare_text(baseline, candidate)
        self.assertIn("| filtered count | 0.000123 | 0.000082 | -33.3% |", summary)

    def test_rejects_incompatible_settings(self):
        with self.assertRaisesRegex(ValueError, "settings do not match"):
            compare_text(report(), report(rows=300000))
        with self.assertRaisesRegex(ValueError, "settings do not match"):
            compare_text(report(), report(samples=7))

    def test_rejects_missing_and_duplicate_workloads(self):
        with self.assertRaisesRegex(ValueError, "missing or unexpected workloads"):
            parse_benchmark(report().replace("filtered sum 0.003 1.00\n", ""))
        with self.assertRaisesRegex(ValueError, "duplicate workload"):
            parse_benchmark(report() + "filtered top-K 1.000 2.00\n")

    def test_rejects_missing_and_duplicate_metadata(self):
        text = report()
        with self.assertRaisesRegex(ValueError, "missing numbers"):
            parse_benchmark("\n".join(text.splitlines()[1:]))
        with self.assertRaisesRegex(ValueError, "duplicate benchmark settings"):
            parse_benchmark(text + text.splitlines()[0] + "\n")


if __name__ == "__main__":
    unittest.main()

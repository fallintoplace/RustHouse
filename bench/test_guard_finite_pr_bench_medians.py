import unittest

from bench.pr_bench_compare import parse_benchmark


def output(median="1.234", rows="8192", samples="7"):
    return (f"RustHouse numbers() benchmarks: {rows} input rows, "
            f"{samples} samples, 3 warmups\n"
            f"filtered count {median} 10.0\n"
            "filtered sum 1.000 11.0\n"
            "filtered top-K 2.000 12.0\n")


class BenchmarkMedianTests(unittest.TestCase):
    def test_rejects_overflowing_float_median(self):
        with self.assertRaisesRegex(ValueError, "invalid benchmark median"):
            parse_benchmark(output("9" * 400 + ".0"))

    def test_rejects_zero_rows_or_samples(self):
        for options in ({"rows": "0"}, {"samples": "0"}):
            with self.subTest(options=options), self.assertRaisesRegex(ValueError, "positive"):
                parse_benchmark(output(**options))

    def test_allows_zero_for_below_printed_precision(self):
        _, median = parse_benchmark(output("0.000"))
        self.assertEqual(median["filtered count"], 0.0)


if __name__ == "__main__":
    unittest.main()

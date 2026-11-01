import unittest
from bench.pr_bench_compare import parse_benchmark


HEADER = "RustHouse numbers() benchmarks: 1000 input rows, 5 samples, 1 warmups\n"
ROWS = "filtered count 1.000 3.000\nfiltered sum 2.000 4.000\nfiltered top-K 3.000 5.000\n"


class NumbersAsciiMeasurementsTests(unittest.TestCase):
    def test_rejects_unicode_numeric_measurements(self):
        self.assertEqual(parse_benchmark(HEADER + ROWS)[0], (1000, 5, 1))
        for malformed in (
            HEADER.replace("1000", "١٠٠٠") + ROWS,
            HEADER + ROWS.replace("1.000", "١.٠٠٠"),
        ):
            with self.subTest(report=malformed), self.assertRaises(ValueError):
                parse_benchmark(malformed)


if __name__ == "__main__":
    unittest.main()

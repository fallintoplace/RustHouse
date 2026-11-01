import unittest
from bench.pr_parts_compare import CASES, parse_benchmark


def sample():
    header = "workload,median_ms,p90_ms,million_logical_rows_per_second\n"
    return header + "".join(f"{name},1.000000,2.000000,3.000000\n" for name in CASES)


class PartsAsciiMeasurementsTests(unittest.TestCase):
    def test_rejects_nonascii_numeric_digits_in_report(self):
        source = sample()
        self.assertEqual(len(parse_benchmark(source)), len(CASES))
        malformed = source.replace("1.000000", "١.٠٠٠٠٠٠", 1)
        with self.assertRaisesRegex(ValueError, "non-ASCII"):
            parse_benchmark(malformed)


if __name__ == "__main__":
    unittest.main()

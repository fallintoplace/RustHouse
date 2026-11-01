import unittest
from bench.pr_grouped_compare import parse_benchmark


HEADER = "RustHouse grouped scans: 65536 rows, 413 keys, 11 samples\n"
ROWS = "min 4.000 5.000 16.38\navg 5.000 6.000 13.11\nmax 6.000 7.000 10.92\n"


class GroupedAsciiMeasurementsTests(unittest.TestCase):
    def test_requires_ascii_digits_in_settings_and_samples(self):
        self.assertEqual(parse_benchmark(HEADER + ROWS)[0], (65536, 413, 11))
        for malformed in (
            HEADER.replace("65536", "٦٥٥٣٦") + ROWS,
            HEADER + ROWS.replace("4.000", "٤.٠٠٠"),
        ):
            with self.subTest(report=malformed), self.assertRaises(ValueError):
                parse_benchmark(malformed)


if __name__ == "__main__":
    unittest.main()

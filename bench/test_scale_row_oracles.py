import unittest

from bench.scale.row_oracles import make_oracles, modulo_reference, sum_range


class RowOracleTests(unittest.TestCase):
    def test_billion_row_even_reference(self):
        report = make_oracles(1_000_000_000)
        self.assertEqual(report["even"], {"count": 500_000_000, "sum": 249_999_999_500_000_000})
        self.assertEqual(report["sum_all"], 499_999_999_500_000_000)

    def test_odd_size_and_empty_subranges(self):
        self.assertEqual(make_oracles(5)["odd"], {"count": 2, "sum": 4})
        self.assertEqual(sum_range(10, 10), 0)
        self.assertEqual(sum_range(3, 6), 12)

    def test_rejects_invalid_moduli(self):
        with self.assertRaisesRegex(ValueError, "modulus"):
            modulo_reference(5, 0, 0)
        with self.assertRaisesRegex(ValueError, "modulus"):
            modulo_reference(5, 3, 3)
        with self.assertRaisesRegex(ValueError, "rows"):
            make_oracles(0)

    def test_large_offset_modulo_oracle(self):
        from bench.scale.row_oracles import modulo_interval_reference
        start = (1 << 64) - 5
        actual = modulo_interval_reference(start, 1 << 64, 2, 0)
        self.assertEqual(actual, {"count": 2, "sum": 2 * (1 << 64) - 6})
        self.assertEqual(modulo_interval_reference(start, start, 2, 1),
                         {"count": 0, "sum": 0})


if __name__ == "__main__":
    unittest.main()

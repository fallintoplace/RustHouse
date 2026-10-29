import unittest

from bench.scale.row_oracles import make_oracles, modulo_reference, sum_range, progression_reference


class RowOracleTests(unittest.TestCase):
    def test_billion_row_even_reference(self):
        report = make_oracles(1_000_000_000)
        self.assertEqual(report["even"], {"count": 500_000_000, "sum": 249_999_999_500_000_000})
        self.assertEqual(report["sum_all"], 499_999_999_500_000_000)

    def test_nonzero_strides_have_exact_parity_sums(self):
        result = make_oracles(5, start=3, step=3)
        self.assertEqual(result["sum_all"], 45)
        self.assertEqual(result["even"], {"count": 2, "sum": 18})
        self.assertEqual(result["odd"], {"count": 3, "sum": 27})
        flat = make_oracles(4, start=2, step=2)
        self.assertEqual(flat["even"], {"count": 4, "sum": 20})
        self.assertEqual(flat["odd"], {"count": 0, "sum": 0})
        self.assertEqual(progression_reference((1 << 64) - 4, 2, 3),
                         {"count": 2, "sum": 2 * (1 << 64) - 5})

    def test_rejects_strided_uint64_overflow(self):
        with self.assertRaisesRegex(ValueError, "overflow"):
            make_oracles(3, start=(1 << 64) - 4, step=2)
        with self.assertRaises(ValueError):
            make_oracles(2, step=0)

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

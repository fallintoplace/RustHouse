import unittest

from bench.scale.row_oracles import make_oracles, modulo_reference, sum_range


class OracleIntegerDomainsTests(unittest.TestCase):
    def test_fractional_values_are_never_valid_exact_oracles(self):
        for fn, args in (
            (sum_range, (0.5, 3)),
            (sum_range, (0, 3.5)),
            (modulo_reference, (3.5, 2, 0)),
            (modulo_reference, (10, 2.5, 0)),
            (modulo_reference, (10, 2, 0.5)),
            (make_oracles, (4.5,)),
            (make_oracles, (True,)),
        ):
            with self.subTest(fn=fn.__name__, args=args), self.assertRaises(ValueError):
                fn(*args)

    def test_valid_integer_oracles_unchanged(self):
        self.assertEqual(modulo_reference(5, 2, 0), {"count": 3, "sum": 6})
        self.assertEqual(sum_range(3, 6), 12)


if __name__ == "__main__":
    unittest.main()

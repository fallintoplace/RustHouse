"""Closed-form modulo SUM(id * id) oracle for huge UInt64 workloads."""
import unittest

from bench.scale.row_oracles import (
    make_oracles, progression_modulo_squares_reference,
)


class ModuloSquareOracleTests(unittest.TestCase):
    def test_matches_brute_force_on_non_coprime_progressions(self):
        for start in (0, 1, 7, 15):
            for step in (1, 2, 3, 6, 11):
                for modulus in range(1, 20):
                    for residue in range(modulus):
                        expected = sum((start + i * step) ** 2 for i in range(30)
                                       if (start + i * step) % modulus == residue)
                        self.assertEqual(
                            progression_modulo_squares_reference(
                                start, 30, step, modulus, residue), expected,
                            (start, step, modulus, residue),
                        )

    def test_max_uint64_and_billion_row_values_remain_exact(self):
        top = (1 << 64) - 1
        expected = (top - 2) ** 2 + top ** 2
        self.assertEqual(
            progression_modulo_squares_reference(top - 2, 2, 2, 2, 1),
            expected,
        )
        report = make_oracles(1_000_000_000, modulus=3, residue=2)
        self.assertEqual(report["modulo_sum_squares"],
                         progression_modulo_squares_reference(
                             0, 1_000_000_000, 1, 3, 2))
        self.assertNotIn("modulo_sum_squares", make_oracles(5))
        with self.assertRaises(ValueError):
            progression_modulo_squares_reference(top, 2, 1, 2, 1)


if __name__ == "__main__":
    unittest.main()

"""Permuted-ID fixture parameters cannot use boolean batch sizes."""
import io
import unittest

from bench.scale.permuted_ids import MASK, MULTIPLIER, emit, permuted


class PermutedFixtureInputTests(unittest.TestCase):
    def test_rejects_boolean_dimensions(self):
        for rows, kwargs in (
            (True, {}),
            (3, {"seed": False}),
            (3, {"batch_rows": True}),
        ):
            with self.subTest(rows=rows, kwargs=kwargs):
                output = io.StringIO()
                with self.assertRaises(ValueError):
                    emit(output, rows, **kwargs)
                self.assertEqual(output.getvalue(), "")
        with self.assertRaises(ValueError):
            permuted(True, 0)
        with self.assertRaises(ValueError):
            permuted(2, False)

    def test_valid_batched_permutation_matches_direct_mapping(self):
        output = io.StringIO()
        emit(output, 7, seed=42, batch_rows=3)
        expected = "".join(f"{permuted(index, 42)}\n" for index in range(7))
        self.assertEqual(output.getvalue(), expected)

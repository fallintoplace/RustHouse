import unittest

from bench.scale.sparse_point_probes import choose_ids, percentile


class SparseProbeNumericBoundTests(unittest.TestCase):
    def test_rejects_invalid_seed_types_and_domains(self):
        for seed in (True, 1.5, "7", -1, 1 << 64):
            with self.subTest(seed=seed), self.assertRaisesRegex(ValueError, "seed"):
                choose_ids(0, 20, 5, seed)

    def test_accepts_highest_uint64_seed_deterministically(self):
        self.assertEqual(choose_ids(10, 100, 20, (1 << 64) - 1),
                         choose_ids(10, 100, 20, (1 << 64) - 1))

    def test_percentile_rejects_invalid_numeric_types(self):
        for percentile_value in (True, "99", float("nan"), float("inf"), 0):
            with self.subTest(percentile=percentile_value), self.assertRaisesRegex(
                    ValueError, "percentile"):
                percentile([1, 2, 3], percentile_value)


if __name__ == "__main__":
    unittest.main()

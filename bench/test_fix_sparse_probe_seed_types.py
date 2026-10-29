import math
import unittest
from bench.scale.sparse_point_probes import choose_ids, percentile

class SparseSeedTests(unittest.TestCase):
    def test_seed_must_be_exact_integer(self):
        for seed in (True, 1.5, "1", None):
            with self.subTest(seed=seed), self.assertRaisesRegex(ValueError, "seed"):
                choose_ids(0, 10, 3, seed)

    def test_rejects_invalid_percentile_types(self):
        for value in (True, "95", math.nan, math.inf, 101):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "percentile"):
                percentile([1, 2, 3], value)

    def test_seed_preserves_reproducibility(self):
        self.assertEqual(choose_ids(10, 100, 20, 42), choose_ids(10, 100, 20, 42))

if __name__ == "__main__":
    unittest.main()

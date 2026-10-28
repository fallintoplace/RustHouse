import math
import unittest

from bench.scale.sparse_point_probes import choose_ids, probe


class SparseProbeInputsTests(unittest.TestCase):
    def test_rejects_fractional_and_boolean_sample_counts(self):
        for count in (True, 1.5, 0):
            with self.subTest(count=count), self.assertRaisesRegex(ValueError, "sample size"):
                choose_ids(0, 10, count, 0)

    def test_rejects_invalid_deadlines_before_query(self):
        for timeout in (math.nan, math.inf, -math.inf, True, 0, -1):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                probe("rusthouse", "db", "events", 0, 10, count=1,
                      timeout=timeout, execute=lambda sql: self.fail("query executed"))

    def test_preserves_deterministic_sampling(self):
        self.assertEqual(choose_ids(0, 10, 3, 42), choose_ids(0, 10, 3, 42))
        self.assertEqual(len(choose_ids(0, 10, 3, 42)), 3)


if __name__ == "__main__":
    unittest.main()

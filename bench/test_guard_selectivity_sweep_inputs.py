import math
import unittest

from bench.scale.selectivity_sweep import run_sweep


class SelectivityInputsTests(unittest.TestCase):
    def test_rejects_invalid_sample_counts_before_query(self):
        for samples in (True, False, 0, -1, 1.5):
            with self.subTest(samples=samples), self.assertRaisesRegex(ValueError, "samples"):
                run_sweep("rusthouse", ".", 8, samples=samples,
                          execute=lambda *args, **kwargs: self.fail("unexpected query"))

    def test_rejects_nonfinite_and_invalid_deadlines(self):
        for timeout in (math.nan, math.inf, -math.inf, True, 0, -5):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                run_sweep("rusthouse", ".", 8, timeout=timeout,
                          execute=lambda *args, **kwargs: self.fail("unexpected query"))


if __name__ == "__main__":
    unittest.main()

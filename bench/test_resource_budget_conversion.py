"""Finite MiB inputs must not create infinite byte limits."""
import math
import sys
import unittest
from unittest.mock import patch

from bench.scale.query_resource_profile import _budget_bytes, profile


class ResourceBudgetOverflowTests(unittest.TestCase):
    def test_rejects_budget_conversion_overflow(self):
        for amount in (10 ** 400, 1e308, float("inf"), float("nan"), True, 0, -1):
            for name in ("max RSS", "max physical read"):
                with self.subTest(amount=str(amount)[:20], name=name):
                    with self.assertRaisesRegex(ValueError, "budget"):
                        _budget_bytes(amount, name)

    def test_uses_exact_finite_megabyte_conversion(self):
        self.assertEqual(_budget_bytes(None, "max RSS"), None)
        self.assertEqual(_budget_bytes(3, "max RSS"), 3 * 1024 * 1024)
        self.assertEqual(_budget_bytes(0.5, "max physical read"), 512 * 1024)
        self.assertTrue(math.isfinite(_budget_bytes(123456, "max RSS")))

    @unittest.skipUnless(sys.platform == "linux", "requires Linux resource sampler")
    @patch("bench.scale.query_resource_profile.subprocess.Popen")
    def test_bad_limits_are_rejected_without_launching_query(self, launch):
        for options in ({"max_rss_mib": 1e308}, {"max_read_mib": 10 ** 400}):
            with self.subTest(options=str(options)[:40]):
                with self.assertRaisesRegex(ValueError, "budget"):
                    profile("binary", ".", "SELECT 1", **options)
        launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()

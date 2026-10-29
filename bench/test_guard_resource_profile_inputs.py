import sys
import unittest

from bench.scale.query_resource_profile import profile


@unittest.skipUnless(sys.platform == "linux", "requires Linux")
class ProfileInputGuardsTests(unittest.TestCase):
    def test_rejects_timeout_and_sample_interval_types_before_launch(self):
        for params in (
            {"timeout": "1"}, {"timeout": True}, {"timeout": float("nan")},
            {"sample_ms": True}, {"sample_ms": 1.5}, {"sample_ms": "25"},
            {"sample_ms": 0}, {"sample_ms": 1001},
        ):
            with self.subTest(params=params), self.assertRaisesRegex(
                    ValueError, "timeout or sample interval"):
                profile("unused", ".", "SELECT 1", **params)

    def test_rejects_invalid_rss_budget_types_before_launch(self):
        for value in (True, "32", float("nan"), float("inf"), 0):
            with self.subTest(value=value), self.assertRaisesRegex(
                    ValueError, "RSS budget"):
                profile("unused", ".", "SELECT 1", max_rss_mib=value)


if __name__ == "__main__":
    unittest.main()

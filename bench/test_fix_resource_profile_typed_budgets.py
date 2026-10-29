import math
import sys
import unittest
from bench.scale.query_resource_profile import profile

@unittest.skipUnless(sys.platform == "linux", "Linux /proc profiler")
class ProfilerBudgetTests(unittest.TestCase):
    def test_bad_types_rejected_before_subprocess(self):
        for kw in ({"sample_ms": True}, {"sample_ms": 1.5}, {"timeout": True},
                   {"timeout": "1"}, {"timeout": math.nan},
                   {"max_rss_mib": True}, {"max_rss_mib": "100"}):
            with self.subTest(kw=kw), self.assertRaises(ValueError):
                profile("nonexistent-rusthouse", ".", "SELECT 1", **kw)

if __name__ == "__main__":
    unittest.main()

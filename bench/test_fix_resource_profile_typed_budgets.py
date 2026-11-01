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

    def test_read_only_validation_accepts_nested_comment_select(self):
        from bench.scale.query_resource_profile import validate_select
        for sql in ("/* outer /* inner */ */ SELECT 1",
                    "-- note\nSELECT 1",
                    "# note\nSELECT(1)"):
            with self.subTest(sql=sql):
                validate_select(sql)
        for sql in ("/* SELECT 1 */ INSERT INTO t VALUES (1)",
                    "SELECT 1; DELETE FROM t",
                    "/* unclosed SELECT 1"):
            with self.subTest(sql=sql), self.assertRaisesRegex(
                    ValueError, "read-only SELECT"):
                validate_select(sql)

if __name__ == "__main__":
    unittest.main()

import math
import unittest
from bench.scale.query_correctness_soak import percentile, soak

class SoakInputTests(unittest.TestCase):
    def test_invalid_percentile_types(self):
        for value in (True, "90", math.inf, math.nan, -1, 101):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "percentile"):
                percentile([1, 2, 3], value)

    def test_invalid_timeouts_fail_before_reference_read(self):
        for value in (True, "30", math.nan, math.inf, 0):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "timeout"):
                soak("rusthouse", "db", "SELECT 1", "/nonexistent.tsv", timeout=value)

    def test_soak_accepts_commented_select_and_keeps_mutations_blocked(self):
        import hashlib
        import tempfile
        from pathlib import Path
        from bench.scale.query_correctness_soak import soak
        with tempfile.TemporaryDirectory() as directory:
            reference = Path(directory) / "reference.tsv"
            reference.write_bytes(b"1\n")
            def execute():
                return hashlib.sha256(b"1\n").hexdigest(), 2, 0.01
            for sql in ("/* note */ SELECT 1",
                        "-- note\nSELECT 1", "SELECT/* comment */ 1"):
                with self.subTest(sql=sql):
                    result = soak("binary", "db", sql, reference,
                                  runs=1, warmups=0, execute=execute)
                    self.assertTrue(result["verified"])
            for sql in ("/* SELECT */ DELETE FROM events",
                        "-- note\nINSERT INTO events VALUES (1)",
                        "SELECT 1; DELETE FROM events"):
                with self.subTest(sql=sql), self.assertRaisesRegex(
                        ValueError, "read-only SELECT"):
                    soak("binary", "db", sql, reference,
                         runs=1, warmups=0, execute=lambda: self.fail("must not execute"))

if __name__ == "__main__":
    unittest.main()

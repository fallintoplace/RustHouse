import hashlib
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.query_correctness_soak import soak


class StableReferenceTests(unittest.TestCase):
    def test_rejects_reference_above_budget_before_query(self):
        with tempfile.TemporaryDirectory() as root:
            oracle = Path(root) / "expected.tsv"
            oracle.write_bytes(b"abcde")
            with patch("bench.scale.query_correctness_soak.MAX_EXPECTED_BYTES", 4):
                with self.assertRaisesRegex(ValueError, "byte limit"):
                    soak("x", "db", "SELECT 1", oracle, runs=1, warmups=0,
                         execute=lambda: self.fail("query must not run"))

    def test_rejects_reference_replaced_before_open(self):
        with tempfile.TemporaryDirectory() as root:
            oracle = Path(root) / "expected.tsv"
            oracle.write_bytes(b"old\n")
            actual_open = os.open

            def replace(path, flags, *args, **kwargs):
                oracle.replace(Path(root) / "moved.tsv")
                oracle.write_bytes(b"new\n")
                return actual_open(path, flags, *args, **kwargs)

            with patch("bench.scale.query_correctness_soak.os.open", side_effect=replace):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    soak("x", "db", "SELECT 1", oracle, runs=1, warmups=0,
                         execute=lambda: self.fail("query must not run"))

    def test_normal_reference_still_matches_every_sample(self):
        with tempfile.TemporaryDirectory() as root:
            oracle = Path(root) / "expected.tsv"
            oracle.write_bytes(b"value\n42\n")
            digest = hashlib.sha256(oracle.read_bytes()).hexdigest()
            report = soak("x", "db", "SELECT 1", oracle, runs=1, warmups=0,
                          execute=lambda: (digest, 9, 0.01))
            self.assertTrue(report["verified"])


if __name__ == "__main__":
    unittest.main()

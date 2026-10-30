"""Regression tests for bounded query correctness soaks."""
import hashlib
import tempfile
import unittest
from pathlib import Path

from bench.scale.query_correctness_soak import soak


class QuerySoakOutputBudgetTests(unittest.TestCase):
    def test_rejects_oversized_reference_before_running(self):
        with tempfile.TemporaryDirectory() as directory:
            reference = Path(directory) / "expected.tsv"
            reference.write_bytes(b"123456")
            calls = []
            with self.assertRaisesRegex(ValueError, "reference output"):
                soak("fake", directory, "SELECT 1", reference, runs=1, warmups=0,
                     max_output_bytes=5, execute=lambda: calls.append(True))
            self.assertFalse(calls)

    def test_rejects_injected_query_output_larger_than_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            reference = Path(directory) / "expected.tsv"
            reference.write_bytes(b"1\\n")
            with self.assertRaisesRegex(ValueError, "query output"):
                soak("fake", directory, "SELECT 1", reference, runs=1, warmups=0,
                     max_output_bytes=2,
                     execute=lambda: ("fake", 3, 0.01))

    def test_accepts_bounded_matching_output(self):
        with tempfile.TemporaryDirectory() as directory:
            reference = Path(directory) / "expected.tsv"
            content = b"1\n"
            reference.write_bytes(content)
            digest = hashlib.sha256(content).hexdigest()
            result = soak("fake", directory, "SELECT 1", reference,
                          runs=2, warmups=1, max_output_bytes=2,
                          execute=lambda: (digest, len(content), 0.02))
            self.assertTrue(result["verified"])
            self.assertEqual(result["runs"], 2)

    def test_rejects_boolean_and_oversized_limits(self):
        with self.assertRaisesRegex(ValueError, "max_output_bytes"):
            soak("fake", ".", "SELECT 1", "none", max_output_bytes=True)
        with self.assertRaisesRegex(ValueError, "max_output_bytes"):
            soak("fake", ".", "SELECT 1", "none", max_output_bytes=1_000_000_001)


if __name__ == "__main__":
    unittest.main()

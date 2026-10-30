import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from bench.product_acceptance import check_rows, invoke

class AcceptanceTests(unittest.TestCase):
    def test_exact_reference(self):
        check_rows("id\towner\tamount\n1\tAda\t20\n2\tGrace\t30\n")

    def test_detects_missing_row(self):
        with self.assertRaises(AssertionError):
            check_rows("id\towner\tamount\n1\tAda\t20\n")

    def test_detects_wrong_value(self):
        with self.assertRaises(AssertionError):
            check_rows("id\towner\tamount\n1\tAda\t20\n2\tGrace\t99\n")

    def test_invoke_rejects_oversized_output(self):
        def write_large_output(_argv, **kwargs):
            kwargs["stdout"].write(b"x" * 65_537)
            return SimpleNamespace(returncode=0)
        with patch("bench.product_acceptance.subprocess.run",
                   side_effect=write_large_output):
            with self.assertRaisesRegex(RuntimeError, "64 KiB"):
                invoke("rusthouse", "sql", "SELECT 1")

    def test_invoke_converts_timeouts_to_runtime_failure(self):
        with patch("bench.product_acceptance.subprocess.run",
                   side_effect=subprocess.TimeoutExpired(["rusthouse"], 120)):
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                invoke("rusthouse", "doctor")

    def test_invoke_uses_bounded_output_and_timeout(self):
        def write_output(_argv, **kwargs):
            self.assertEqual(kwargs["timeout"], 120)
            kwargs["stdout"].write(b"ok\\n")
            return SimpleNamespace(returncode=0)
        with patch("bench.product_acceptance.subprocess.run",
                   side_effect=write_output):
            self.assertEqual(invoke("rusthouse", "sql", "SELECT 1"), "ok\\n")


if __name__ == "__main__":
    unittest.main()

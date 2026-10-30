"""Bound live subprocess output without changing injected-runner semantics."""
import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.billion_matrix import _invoke


class QueryMatrixOutputTests(unittest.TestCase):
    def test_spools_live_output(self):
        def run(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            kwargs["stdout"].write(b"3\n")
            return SimpleNamespace(returncode=0)
        with patch("bench.scale.billion_matrix.subprocess.run", side_effect=run):
            result, elapsed = _invoke(["fake"], timeout=3)
        self.assertEqual(result, b"3\n")
        self.assertGreaterEqual(elapsed, 0)

    def test_bounds_stderr_tail(self):
        def run(command, **kwargs):
            kwargs["stderr"].write(b"x" * 100_000 + b"last error")
            return SimpleNamespace(returncode=1)
        with patch("bench.scale.billion_matrix.subprocess.run", side_effect=run):
            with self.assertRaises(RuntimeError) as caught:
                _invoke(["fake"], timeout=3)
        self.assertIn("last error", str(caught.exception))
        self.assertLess(len(str(caught.exception)), 640)

    def test_rejects_unbounded_stdout(self):
        def run(command, **kwargs):
            kwargs["stdout"].truncate(8 * 1024 * 1024 + 1)
            return SimpleNamespace(returncode=0)
        with patch("bench.scale.billion_matrix.subprocess.run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "8 MiB"):
                _invoke(["fake"], timeout=3)

    def test_injected_execution_keeps_captured_output_contract(self):
        def fake(command, **kwargs):
            self.assertTrue(kwargs["capture_output"])
            return subprocess.CompletedProcess(command, 0, stdout=b"42\n", stderr=b"")
        output, _ = _invoke(["fake"], timeout=1, execute=fake)
        self.assertEqual(output, b"42\n")


if __name__ == "__main__":
    unittest.main()

"""Top-K result spooling is constrained before child output is produced."""
import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.topk_probe import probe


class TopKOnlineOutputCapTests(unittest.TestCase):
    @patch("bench.scale.topk_probe.subprocess.run")
    def test_both_directions_use_bounded_child_output(self, run):
        commands = []
        def execute(command, **kwargs):
            commands.append(command)
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            else:
                self.assertNotIn("preexec_fn", kwargs)
            output = b"0\n1\n" if "ASC" in command[2] else b"9\n8\n"
            kwargs["stdout"].write(output)
            return SimpleNamespace(returncode=0)
        run.side_effect = execute
        report = probe("binary", "part", 10, limit=2, timeout=5)
        self.assertEqual(len(commands), 2)
        self.assertEqual([x["direction"] for x in report["results"]], ["ASC", "DESC"])

    @patch("bench.scale.topk_probe.subprocess.run")
    def test_oversized_result_fails_reference_verification(self, run):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"1" * 1024)
            return SimpleNamespace(returncode=0)
        run.side_effect = execute
        with self.assertRaisesRegex(ValueError, "top-K differs"):
            probe("binary", "part", 10, limit=1)


if __name__ == "__main__":
    unittest.main()

import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.topk_probe import probe


class TopKOutputBudgetTests(unittest.TestCase):
    def test_real_subprocess_path_caps_stdout_on_posix(self):
        def run(command, **kwargs):
            sql = command[2]
            self.assertTrue(callable(kwargs.get("preexec_fn")) if os.name == "posix"
                            else "preexec_fn" not in kwargs)
            output = b"2\n1\n" if "DESC" in sql else b"0\n1\n"
            kwargs["stdout"].write(output)
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.topk_probe.subprocess.run", side_effect=run):
            report = probe("rusthouse", "part", 3, limit=2)
        self.assertTrue(all(result["verified"] for result in report["results"]))

    def test_oversized_spooled_stdout_never_passes(self):
        def run(_command, **kwargs):
            kwargs["stdout"].write(b"0\n1\n" + b"x" * 10000)
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.topk_probe.subprocess.run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "differs"):
                probe("rusthouse", "part", 3, limit=2)


if __name__ == "__main__":
    unittest.main()

import math
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from bench.common import cli_sql

class SharedRunnerTimeoutTests(unittest.TestCase):
    @patch("bench.common.subprocess.run")
    def test_invalid_timeout_never_starts_process(self, run):
        for timeout in (True, "30", math.nan, math.inf, 0, -1):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "timeout"):
                cli_sql("rusthouse", "db", "SELECT 1", timeout)
        run.assert_not_called()

    @patch("bench.common.subprocess.run")
    def test_valid_timeout_reaches_process(self, run):
        def execute(command, **kwargs):
            kwargs["stdout"].write(b"1\n")
            return SimpleNamespace(returncode=0, stderr=b"")
        run.side_effect = execute
        self.assertEqual(cli_sql("rusthouse", "db", "SELECT 1", 1), b"1\n")
        self.assertEqual(run.call_args.kwargs["timeout"], 1)

if __name__ == "__main__":
    unittest.main()

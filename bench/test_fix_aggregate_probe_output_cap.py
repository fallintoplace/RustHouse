"""Aggregate probe subprocesses must have bounded spooled output."""
import os
import subprocess
import unittest
from unittest import mock

from bench.scale import aggregate_probe


class AggregateSpoolLimitTests(unittest.TestCase):
    def test_real_subprocess_path_sets_a_posix_file_limit(self):
        expected = dict(aggregate_probe.aggregates(3))
        def run(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            expression = command[2].removeprefix("SELECT ").removesuffix(" FROM part")
            kwargs["stdout"].write(f"{expected[expression]}\n".encode())
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(aggregate_probe.subprocess, "run", side_effect=run) as runner:
            result = aggregate_probe.verify("rusthouse", "part", 3)
        self.assertEqual(runner.call_count, 4)
        self.assertEqual(len(result["results"]), 4)

    @unittest.skipUnless(os.name == "posix", "POSIX file size controls only")
    def test_limit_respects_hard_file_limit(self):
        import resource
        expected = dict(aggregate_probe.aggregates(1))
        def run(command, **kwargs):
            with mock.patch.object(resource, "getrlimit", return_value=(200, 4096)), \
                 mock.patch.object(resource, "setrlimit") as set_limit:
                kwargs["preexec_fn"]()
                set_limit.assert_called_once_with(resource.RLIMIT_FSIZE, (4096, 4096))
            expression = command[2].removeprefix("SELECT ").removesuffix(" FROM part")
            kwargs["stdout"].write(f"{expected[expression]}\n".encode())
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(aggregate_probe.subprocess, "run", side_effect=run):
            aggregate_probe.verify("rusthouse", "part", 1)


if __name__ == "__main__":
    unittest.main()

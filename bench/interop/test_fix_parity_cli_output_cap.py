"""CLI/HTTP parity must cap subprocess output at the boundary."""
import os
import subprocess
import unittest
from unittest import mock

from bench.interop import cli_http_parity as parity


class ParityOutputLimitTests(unittest.TestCase):
    def test_parity_cli_installs_child_limit_without_changing_output(self):
        def run(command, **kwargs):
            self.assertNotIn("capture_output", kwargs)
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            kwargs["stdout"].write(b"count\n1\n")
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(parity.subprocess, "run", side_effect=run):
            self.assertEqual(parity.cli("rusthouse", "db", "SELECT count(*)"), b"count\n1\n")

    @unittest.skipUnless(os.name == "posix", "POSIX file limits only")
    def test_file_cap_matches_existing_four_mib_result_check(self):
        import resource
        def run(command, **kwargs):
            with mock.patch.object(resource, "getrlimit", return_value=(resource.RLIM_INFINITY, resource.RLIM_INFINITY)), \
                 mock.patch.object(resource, "setrlimit") as set_limit:
                kwargs["preexec_fn"]()
                set_limit.assert_called_once_with(
                    resource.RLIMIT_FSIZE, (4 * 1024 * 1024 + 1, resource.RLIM_INFINITY))
            kwargs["stdout"].write(b"1\n")
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(parity.subprocess, "run", side_effect=run):
            self.assertEqual(parity.cli("rusthouse", "db", "SELECT 1"), b"1\n")


if __name__ == "__main__":
    unittest.main()

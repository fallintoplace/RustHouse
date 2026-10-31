"""Physical scans must cap child file output before execution."""
import os
import subprocess
import unittest
from unittest import mock

from bench.scale import physical_scan


class PhysicalScanSpoolLimitTests(unittest.TestCase):
    def test_output_cap_is_installed_for_real_processes(self):
        def run(command, **kwargs):
            self.assertEqual(kwargs["timeout"], 30)
            self.assertNotIn("capture_output", kwargs)
            if os.name == "posix":
                self.assertTrue(callable(kwargs.get("preexec_fn")))
            kwargs["stdout"].write(b"3\n")
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(physical_scan.subprocess, "run", side_effect=run):
            output, _duration = physical_scan.invoke("rusthouse", "db", "SELECT 3", timeout=30)
        self.assertEqual(output, b"3\n")

    @unittest.skipUnless(os.name == "posix", "POSIX file limits only")
    def test_cap_is_one_byte_beyond_the_existing_result_budget(self):
        import resource
        def run(command, **kwargs):
            with mock.patch.object(resource, "getrlimit", return_value=(resource.RLIM_INFINITY, resource.RLIM_INFINITY)), \
                 mock.patch.object(resource, "setrlimit") as set_limit:
                kwargs["preexec_fn"]()
                set_limit.assert_called_once_with(
                    resource.RLIMIT_FSIZE, (8 * 1024 * 1024 + 1, resource.RLIM_INFINITY))
            kwargs["stdout"].write(b"3\n")
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.object(physical_scan.subprocess, "run", side_effect=run):
            self.assertEqual(physical_scan.invoke("rusthouse", "db", "SELECT 3", timeout=30)[0], b"3\n")


if __name__ == "__main__":
    unittest.main()

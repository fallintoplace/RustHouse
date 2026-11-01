import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.physical_range_oracle import MAX_ORACLE_RESULT_BYTES, invoke


class PhysicalOracleOutputCapTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "POSIX RLIMIT_FSIZE")
    def test_child_cannot_write_unbounded_query_output(self):
        import resource
        def fake_run(_args, **kwargs):
            self.assertIn("preexec_fn", kwargs)
            with patch.object(resource, "getrlimit",
                              return_value=(resource.RLIM_INFINITY, resource.RLIM_INFINITY)):
                with patch.object(resource, "setrlimit") as set_limit:
                    kwargs["preexec_fn"]()
            set_limit.assert_called_once_with(
                resource.RLIMIT_FSIZE,
                (MAX_ORACLE_RESULT_BYTES + 1, resource.RLIM_INFINITY))
            kwargs["stdout"].write(b"ok\n")
            return SimpleNamespace(returncode=0)

        with patch("bench.scale.physical_range_oracle.subprocess.run", side_effect=fake_run):
            self.assertEqual(invoke("binary", "db", "SELECT 1", 10), b"ok\n")

    def test_output_budget_wins_over_nonzero_exit_code(self):
        def fake_run(_args, **kwargs):
            kwargs["stdout"].write(b"x" * (MAX_ORACLE_RESULT_BYTES + 1))
            return SimpleNamespace(returncode=1)
        with patch("bench.scale.physical_range_oracle.subprocess.run", side_effect=fake_run):
            with self.assertRaisesRegex(ValueError, "1 KiB"):
                invoke("binary", "db", "SELECT 1", 10)


if __name__ == "__main__":
    unittest.main()

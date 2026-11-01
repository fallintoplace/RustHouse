import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bench.common import file_change_time_ns
from bench import compare
from bench.analytics import run, suite_runner


class WindowsFileIdentityTests(unittest.TestCase):
    def test_win_birthtime_is_stable_when_ctime_differs(self):
        first = SimpleNamespace(st_birthtime_ns=101, st_ctime_ns=202)
        next_read = SimpleNamespace(st_birthtime_ns=101, st_ctime_ns=303)
        with patch("bench.common.os.name", "nt"):
            self.assertEqual(file_change_time_ns(first), file_change_time_ns(next_read))

    def test_posix_changes_use_ctime(self):
        first = SimpleNamespace(st_birthtime_ns=101, st_ctime_ns=202)
        next_read = SimpleNamespace(st_birthtime_ns=101, st_ctime_ns=303)
        with patch("bench.common.os.name", "posix"):
            self.assertNotEqual(file_change_time_ns(first), file_change_time_ns(next_read))

    def test_benchmark_loaders_reuse_the_portable_metadata_clock(self):
        self.assertIs(compare.file_change_time_ns, file_change_time_ns)
        self.assertIs(run.file_change_time_ns, file_change_time_ns)
        self.assertIs(suite_runner.file_change_time_ns, file_change_time_ns)


if __name__ == "__main__":
    unittest.main()

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

    def test_benchmark_loaders_use_portable_metadata_clock(self):
        # Some scripts import "common" independently of "bench.common",
        # so compare behavior rather than Python function identity.
        before = SimpleNamespace(st_birthtime_ns=201, st_ctime_ns=202)
        after = SimpleNamespace(st_birthtime_ns=201, st_ctime_ns=203)
        with patch("bench.common.os.name", "nt"):
            for loader in (compare, run, suite_runner):
                with self.subTest(loader=loader.__name__):
                    self.assertEqual(
                        loader.file_change_time_ns(before),
                        loader.file_change_time_ns(after),
                    )


if __name__ == "__main__":
    unittest.main()

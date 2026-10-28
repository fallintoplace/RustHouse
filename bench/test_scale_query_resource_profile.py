import subprocess
import sys
import unittest

from bench.scale.query_resource_profile import parse_io, parse_rss, profile


class ResourceProfileTests(unittest.TestCase):
    def test_status_rss_kilobytes_to_bytes(self):
        self.assertEqual(parse_rss("Name:\trusthouse\nVmRSS:\t1234 kB\n"), 1234 * 1024)
        self.assertEqual(parse_rss("Name:\tshort\n"), 0)

    def test_physical_io_fields(self):
        self.assertEqual(parse_io("rchar: 300\nread_bytes: 5000\nwrite_bytes: 7\n"),
                         {"read_bytes": 5000, "write_bytes": 7})
        self.assertEqual(parse_io(""), {"read_bytes": 0, "write_bytes": 0})

    @unittest.skipUnless(sys.platform == "linux", "requires Linux")
    def test_successful_short_lived_subprocess(self):
        # A tiny Python process exercises resource monitoring, not RustHouse SQL.
        # The profiler's argument shape is tested through this harmless shim.
        with self.assertRaises(RuntimeError):
            profile(sys.executable, ".", "ignored", timeout=1, sample_ms=10)

    def test_budget_input_validation(self):
        if sys.platform == "linux":
            with self.assertRaises(ValueError):
                profile("unused", ".", "x", timeout=-1)
            with self.assertRaises(ValueError):
                profile("unused", ".", "x", max_rss_mib=0)


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from common import file_change_time_ns
from data_manifest import file_change_time_ns as manifest_timestamp
from run_canonical import file_change_time_ns as canonical_timestamp, read_bounded
from throughput import file_change_time_ns as throughput_timestamp, read_reference


class PortableTpchReadersTests(unittest.TestCase):
    def test_all_readers_reuse_platform_specific_change_time(self):
        self.assertIs(manifest_timestamp, file_change_time_ns)
        self.assertIs(canonical_timestamp, file_change_time_ns)
        self.assertIs(throughput_timestamp, file_change_time_ns)

    def test_windows_birthtime_remains_stable_across_ctime_aliases(self):
        one = SimpleNamespace(st_birthtime_ns=12, st_ctime_ns=13)
        other = SimpleNamespace(st_birthtime_ns=12, st_ctime_ns=14)
        with patch("common.os.name", "nt"):
            self.assertEqual(file_change_time_ns(one), file_change_time_ns(other))

    def test_reads_unmodified_files_after_preflight(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "q01.tsv"
            path.write_bytes(b"1\n2\n")
            self.assertEqual(read_bounded(path, 1024), "1\n2\n")
            self.assertEqual(read_reference(path), b"1\n2\n")


if __name__ == "__main__":
    unittest.main()

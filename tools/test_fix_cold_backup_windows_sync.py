import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tools.offline_backup import _metadata_change_time_ns, sync_copied_tree


class PortableColdBackupSyncTests(unittest.TestCase):
    def test_fsync_existing_copied_file_on_all_platforms(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "table.data"
            target.write_bytes(b"verified rows\n")
            sync_copied_tree(Path(root))
            self.assertEqual(target.read_bytes(), b"verified rows\n")

    def test_change_time_uses_windows_birthtime(self):
        first = SimpleNamespace(st_ctime_ns=1, st_birthtime_ns=42)
        second = SimpleNamespace(st_ctime_ns=2, st_birthtime_ns=42)
        with patch("tools.offline_backup.os.name", "nt"):
            self.assertEqual(_metadata_change_time_ns(first), _metadata_change_time_ns(second))
        with patch("tools.offline_backup.os.name", "posix"):
            self.assertNotEqual(_metadata_change_time_ns(first), _metadata_change_time_ns(second))


if __name__ == "__main__":
    unittest.main()

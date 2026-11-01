import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tools.offline_vacuum import _metadata_identity, active_label


class WindowsVacuumPointerIdentityTests(unittest.TestCase):
    def test_windows_ctime_alias_does_not_reject_same_birth_time(self):
        before = SimpleNamespace(st_dev=5, st_ino=6, st_size=3,
                                 st_mtime_ns=100, st_birthtime_ns=7, st_ctime_ns=8)
        after = SimpleNamespace(st_dev=5, st_ino=6, st_size=3,
                                st_mtime_ns=100, st_birthtime_ns=7, st_ctime_ns=9)
        with patch("tools.offline_vacuum.os.name", "nt"):
            self.assertEqual(_metadata_identity(before), _metadata_identity(after))

    def test_windows_birthtime_and_posix_ctime_still_detect_changes(self):
        before = SimpleNamespace(st_dev=5, st_ino=6, st_size=3,
                                 st_mtime_ns=100, st_birthtime_ns=7, st_ctime_ns=8)
        changed = SimpleNamespace(st_dev=5, st_ino=6, st_size=3,
                                  st_mtime_ns=100, st_birthtime_ns=9, st_ctime_ns=9)
        with patch("tools.offline_vacuum.os.name", "nt"):
            self.assertNotEqual(_metadata_identity(before), _metadata_identity(changed))
        with patch("tools.offline_vacuum.os.name", "posix"):
            self.assertNotEqual(_metadata_identity(before), _metadata_identity(changed))

    def test_active_label_reads_canonical_pointer(self):
        with tempfile.TemporaryDirectory() as root:
            parts = Path(root)
            (parts / ".rusthouse-active").write_bytes(b"g7\n")
            self.assertEqual(active_label(parts), "g7")


if __name__ == "__main__":
    unittest.main()

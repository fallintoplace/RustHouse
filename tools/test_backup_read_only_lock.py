"""Cold backups should lock an existing inode on read-only mounts."""
import errno
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.offline_backup import fcntl, refuse_live_database


@unittest.skipIf(fcntl is None or sys.platform == "win32",
                 "POSIX flock is required")
class ReadOnlyBackupLockTests(unittest.TestCase):
    def test_existing_lock_can_be_opened_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            lock = source / ".rusthouse-write.lock"
            lock.write_bytes(b"")
            actual_open = os.open

            def simulate_read_only(path, flags, *args, **kwargs):
                if Path(path) == lock and flags & os.O_RDWR:
                    raise OSError(errno.EROFS, "read-only filesystem")
                return actual_open(path, flags, *args, **kwargs)

            with patch("tools.offline_backup.os.open", side_effect=simulate_read_only):
                with refuse_live_database(source):
                    with lock.open("rb") as competing:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(competing.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertTrue(lock.is_file())

    def test_missing_lock_must_not_be_silently_bypassed(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            lock = source / ".rusthouse-write.lock"
            actual_open = os.open

            def simulate_read_only(path, flags, *args, **kwargs):
                if Path(path) == lock and flags & os.O_RDWR:
                    raise OSError(errno.EROFS, "read-only filesystem")
                return actual_open(path, flags, *args, **kwargs)

            with patch("tools.offline_backup.os.open", side_effect=simulate_read_only):
                with self.assertRaises(FileNotFoundError):
                    with refuse_live_database(source):
                        self.fail("unlocked backup must not run")
            self.assertFalse(lock.exists())


if __name__ == "__main__":
    unittest.main()

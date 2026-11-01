from contextlib import contextmanager
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from offline_backup import backup, preflight


class BackupLockedPreflightTests(unittest.TestCase):
    def test_tree_validation_happens_while_advisory_lock_is_held(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "db"
            source.mkdir()
            (source / "data.bin").write_bytes(b"snapshot data")
            destination = Path(tmp) / "backup"
            locked = False

            @contextmanager
            def tracked_lock(_source):
                nonlocal locked
                locked = True
                try:
                    yield
                finally:
                    locked = False

            def validate_under_lock(*args, **kwargs):
                self.assertTrue(locked, "source preflight must run under the backup lock")
                return preflight(*args, **kwargs)

            with patch("offline_backup.refuse_live_database", side_effect=tracked_lock), patch(
                "offline_backup.preflight", side_effect=validate_under_lock
            ), patch("offline_backup.subprocess.run",
                     return_value=SimpleNamespace(returncode=0)):
                actual = backup(source, destination, binary="unused")
            self.assertEqual(actual, destination)
            self.assertEqual((destination / "data.bin").read_bytes(), b"snapshot data")
            self.assertFalse(locked)


if __name__ == "__main__":
    unittest.main()

import sys
import tempfile
import unittest
from pathlib import Path

from tools.offline_backup import preflight


class BackupSymlinkTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows CI does not allow unprivileged symlinks")
    def test_rejects_dangling_destination_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "database"
            source.mkdir()
            dest = root / "backup"
            dest.symlink_to(root / "missing")
            with self.assertRaisesRegex(ValueError, "symlink"):
                preflight(source, dest)

    @unittest.skipIf(sys.platform == "win32", "Windows CI does not allow unprivileged symlinks")
    def test_rejects_dangling_staging_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "database"
            source.mkdir()
            stage = root / "backup.partial"
            stage.symlink_to(root / "missing")
            with self.assertRaisesRegex(ValueError, "staging destination"):
                preflight(source, root / "backup")


if __name__ == "__main__":
    unittest.main()

import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from offline_backup import backup


class BackupSymlinkRaceTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "requires POSIX symlinks")
    def test_symlink_inserted_after_preflight_is_not_dereferenced(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            source = base / "db"
            source.mkdir()
            (source / "part.bin").write_bytes(b"safe")
            secret = base / "outside-secret"
            secret.write_bytes(b"never-copy-me")
            destination = base / "archive"
            original_copytree = shutil.copytree

            def raced_copy(source_path, stage_path, **kwargs):
                self.assertTrue(kwargs["symlinks"])
                (source_path / "raced-link").symlink_to(secret)
                return original_copytree(source_path, stage_path, **kwargs)

            with patch("offline_backup.shutil.copytree", side_effect=raced_copy):
                with self.assertRaisesRegex(ValueError, "symlinks"):
                    backup(source, destination)
            self.assertFalse(destination.exists())
            staged = base / "archive.partial" / "raced-link"
            self.assertTrue(staged.is_symlink())
            self.assertEqual(staged.readlink(), secret)


if __name__ == "__main__":
    unittest.main()

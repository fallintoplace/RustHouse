import tempfile
import unittest
from pathlib import Path
from offline_backup import preflight

class BackupTests(unittest.TestCase):
    def test_rejects_nested_destination(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)
            with self.assertRaises(ValueError):
                preflight(source, source / "backup")

    def test_rejects_existing_destination(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                preflight(Path(root), Path(root))

    def test_rejects_symlink_sources(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            (folder / "link").symlink_to(folder)
            with self.assertRaises(ValueError):
                preflight(folder, folder.parent / "unused-backup-for-test")

    def test_rejects_backup_exceeding_explicit_disk_budget(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "database"
            source.mkdir()
            (source / "part.bin").write_bytes(b"abcde")
            (source / "metadata.txt").write_bytes(b"12")
            destination = Path(root) / "backup"
            with self.assertRaisesRegex(ValueError, "max_bytes"):
                preflight(source, destination, max_bytes=6)
            self.assertEqual(preflight(source, destination, max_bytes=7)[1], destination)
            self.assertFalse(destination.exists())
            with self.assertRaisesRegex(ValueError, "max_bytes"):
                preflight(source, destination, max_bytes=0)

    @unittest.skipUnless(hasattr(__import__("os"), "mkfifo"), "FIFO is POSIX-only")
    def test_rejects_fifo_before_copytree_can_block(self):
        import os
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            os.mkfifo(source / "reads.fifo")
            with self.assertRaisesRegex(ValueError, "regular files"):
                preflight(source, Path(root) / "backup")

    def test_allows_fresh_sibling(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            dest = Path(root) / "archive"
            self.assertEqual(preflight(source, dest)[1], dest)


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path
from offline_backup import backup, preflight
from unittest.mock import patch
from types import SimpleNamespace

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

    def test_rejects_backup_metadata_explosion(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            source = base / "db"
            source.mkdir()
            for number in range(4):
                (source / f"part{number}").write_bytes(b"x")
            destination = base / "backup"
            with self.assertRaisesRegex(ValueError, "max_entries"):
                preflight(source, destination, max_entries=3)
            self.assertEqual(preflight(source, destination, max_entries=4)[1], destination)
            with self.assertRaisesRegex(ValueError, "max_entries"):
                preflight(source, destination, max_entries=True)

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

    def test_failed_doctor_spools_and_bounds_stderr(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            source = base / "db"
            source.mkdir()
            (source / "part.bin").write_bytes(b"data")
            def reject(_command, **kwargs):
                self.assertEqual(kwargs["stdout"], __import__("subprocess").DEVNULL)
                kwargs["stderr"].write(b"x" * 100_000 + b"reason")
                return SimpleNamespace(returncode=1)
            with patch("offline_backup.subprocess.run", side_effect=reject):
                with self.assertRaisesRegex(RuntimeError, "reason") as raised:
                    backup(source, base / "backup")
            self.assertLess(len(str(raised.exception)), 2300)
            self.assertTrue((base / "backup.partial").exists())
            self.assertFalse((base / "backup").exists())

    def test_allows_fresh_sibling(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            dest = Path(root) / "archive"
            self.assertEqual(preflight(source, dest)[1], dest)


    def test_verification_timeout_is_configurable(self):
        from offline_backup import backup
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            (source / "part.bin").write_bytes(b"test")
            destination = Path(root) / "backup"
            with patch("offline_backup.subprocess.run") as run:
                run.return_value.returncode = 0
                run.return_value.stderr = ""
                self.assertEqual(
                    backup(source, destination, doctor_timeout=12), destination
                )
                self.assertEqual(run.call_args.kwargs["timeout"], 12)
            self.assertTrue(destination.exists())
            with self.assertRaisesRegex(ValueError, "doctor_timeout"):
                backup(source, Path(root) / "invalid", doctor_timeout=float("inf"))

if __name__ == "__main__":
    unittest.main()

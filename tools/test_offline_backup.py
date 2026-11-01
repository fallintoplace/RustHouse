import tempfile
import unittest
from pathlib import Path
from offline_backup import backup, preflight
from unittest.mock import patch
from types import SimpleNamespace

class BackupTests(unittest.TestCase):
    def test_directory_walk_errors_abort_validation_and_sync(self):
        from offline_backup import sync_copied_tree, validate_tree
        def incomplete_walk(*_args, **kwargs):
            kwargs["onerror"](PermissionError("unreadable child directory"))
            yield ()
        with patch("offline_backup.os.walk", side_effect=incomplete_walk):
            for operation in (validate_tree, sync_copied_tree):
                with self.subTest(operation=operation), self.assertRaisesRegex(
                    PermissionError, "unreadable child directory"
                ):
                    operation(Path("/unreadable"))

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

    @unittest.skipUnless(__import__("os").name == "posix", "POSIX flock required")
    def test_missing_flock_support_fails_closed(self):
        from offline_backup import refuse_live_database
        with tempfile.TemporaryDirectory() as folder:
            with patch("offline_backup.fcntl", None):
                with self.assertRaisesRegex(ValueError, "requires POSIX advisory"):
                    with refuse_live_database(Path(folder)):
                        self.fail("backup must not run without its database lock")

    @unittest.skipUnless(__import__("os").name == "posix", "flock is POSIX-only")
    def test_refuses_a_live_database_lock(self):
        import fcntl
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            source = folder / "db"
            source.mkdir()
            lock = source / ".rusthouse-write.lock"
            lock.write_bytes(b"")
            with lock.open("r+b") as owned:
                fcntl.flock(owned, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with self.assertRaisesRegex(ValueError, "database is in use"):
                    backup(source, folder / "copy")
            self.assertFalse((folder / "copy.partial").exists())


    @unittest.skipUnless(__import__("os").name == "posix", "flock is POSIX-only")
    def test_holds_newly_created_database_lock_during_copy(self):
        import fcntl
        from offline_backup import refuse_live_database
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            lock = source / ".rusthouse-write.lock"
            self.assertFalse(lock.exists())
            with refuse_live_database(source):
                self.assertTrue(lock.is_file())
                with lock.open("r+b") as concurrent:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(concurrent, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with lock.open("r+b") as later:
                fcntl.flock(later, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_rechecks_backup_size_after_copy(self):
        import shutil
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "db"
            source.mkdir()
            (source / "part.bin").write_bytes(b"abc")
            destination = Path(root) / "backup"
            original_copytree = shutil.copytree

            def growing_copy(*args, **kwargs):
                copied = original_copytree(*args, **kwargs)
                (copied / "part.bin").write_bytes(b"0123456789")
                return copied

            with patch("offline_backup.shutil.copytree", side_effect=growing_copy):
                with self.assertRaisesRegex(ValueError, "max_bytes"):
                    backup(source, destination, max_bytes=4)
            self.assertFalse(destination.exists())
            self.assertTrue(Path(str(destination) + ".partial").exists())


    def test_syncs_file_and_directory_before_publication(self):
        from offline_backup import sync_copied_tree
        import os
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "stage"
            nested = folder / "parts"
            nested.mkdir(parents=True)
            (nested / "count.txt").write_bytes(b"1\\n")
            calls = []
            original = os.fsync
            def counted(fd):
                calls.append(fd)
                return original(fd)
            with patch("offline_backup.os.fsync", side_effect=counted):
                sync_copied_tree(folder)
            self.assertGreaterEqual(len(calls), 3 if os.name == "posix" else 1)


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

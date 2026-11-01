import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from bench.scale.snapshot_fingerprint import fingerprint


class SnapshotFingerprintTests(unittest.TestCase):
    def test_deterministic_and_change_sensitive(self):
        with tempfile.TemporaryDirectory() as root:
            a = Path(root) / "a.bin"
            b = Path(root) / "z.txt"
            a.write_bytes(b"one")
            b.write_bytes(b"two")
            first = fingerprint(root)
            self.assertEqual(first, fingerprint(root))
            self.assertEqual(first["bytes"], 6)
            b.write_bytes(b"three")
            self.assertNotEqual(first["sha256"], fingerprint(root)["sha256"])

    def test_gates_fingerprint_on_exact_hash(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "data.bin").write_bytes(b"trusted")
            sha = fingerprint(root)["sha256"]
            result = fingerprint(root, expected_sha256=sha)
            self.assertTrue(result["verified"])
            with self.assertRaisesRegex(ValueError, "differs"):
                fingerprint(root, expected_sha256="0" * 64)
            with self.assertRaisesRegex(ValueError, "expected_sha256"):
                fingerprint(root, expected_sha256="0" * 63)

    def test_empty_directory_changes_snapshot_identity(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            (base / "data.bin").write_bytes(b"unchanged")
            original = fingerprint(base)
            (base / "empty").mkdir()
            with_empty = fingerprint(base)
            self.assertNotEqual(original["sha256"], with_empty["sha256"])
            self.assertEqual(original["files"], with_empty["files"])
            self.assertEqual(original["bytes"], with_empty["bytes"])
            self.assertEqual(with_empty, fingerprint(base))
            (base / "empty").rmdir()
            self.assertEqual(original, fingerprint(base))

    def test_rejects_empty_directory(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "no files"):
                fingerprint(root)

    def test_unreadable_subtree_aborts_fingerprint(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "root.txt").write_bytes(b"present")

            def fail(directory, *, followlinks, onerror):
                onerror(PermissionError("cannot read nested directory"))
                return iter(())

            with patch("bench.scale.snapshot_fingerprint.os.walk", side_effect=fail):
                with self.assertRaisesRegex(PermissionError, "cannot read"):
                    fingerprint(root)

    def test_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as root:
            a = Path(root) / "a.bin"
            a.write_bytes(b"one")
            (Path(root) / "link").symlink_to(a)
            with self.assertRaisesRegex(ValueError, "nonregular"):
                fingerprint(root)

    def test_rejects_file_identity_change_during_open(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "data.bin").write_bytes(b"one")
            with patch("bench.scale.snapshot_fingerprint.os.fstat",
                       return_value=SimpleNamespace(st_dev=-1, st_ino=-1)):
                with self.assertRaisesRegex(ValueError, "replaced while opening"):
                    fingerprint(root)


    def test_regular_files_are_opened_with_nofollow_on_posix(self):
        import os
        if not getattr(os, "O_NOFOLLOW", 0):
            self.skipTest("O_NOFOLLOW unavailable")
        real_open = os.open
        checked = []

        def nofollow_open(path, flags, *args, **kwargs):
            if Path(path).name == "data.bin":
                checked.append(bool(flags & os.O_NOFOLLOW))
            return real_open(path, flags, *args, **kwargs)

        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "data.bin").write_bytes(b"payload")
            with patch("bench.scale.snapshot_fingerprint.os.open", side_effect=nofollow_open):
                fingerprint(root)
        self.assertEqual(checked, [True])

    def test_detects_same_size_file_metadata_change_during_hashing(self):
        import os
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "data.bin").write_bytes(b"1234")
            original_fstat = os.fstat
            calls = 0

            def changed_ctime(fd):
                nonlocal calls
                result = original_fstat(fd)
                calls += 1
                if calls == 2:
                    return SimpleNamespace(
                        st_dev=result.st_dev, st_ino=result.st_ino,
                        st_size=result.st_size, st_mtime_ns=result.st_mtime_ns,
                        st_ctime_ns=result.st_ctime_ns + 1,
                    )
                return result

            with patch("bench.scale.snapshot_fingerprint.os.fstat", side_effect=changed_ctime):
                with self.assertRaisesRegex(ValueError, "changed during hashing"):
                    fingerprint(root)

    def test_hash_is_independent_of_configurable_read_block_size(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "payload.bin").write_bytes(b"abcd" * 50_000)
            small = fingerprint(root, block_size=4096)
            large = fingerprint(root, block_size=16 * 1024 * 1024)
            self.assertEqual(small["sha256"], large["sha256"])
            with self.assertRaisesRegex(ValueError, "block_size"):
                fingerprint(root, block_size=4095)

if __name__ == "__main__":
    unittest.main()

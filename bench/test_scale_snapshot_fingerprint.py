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


if __name__ == "__main__":
    unittest.main()

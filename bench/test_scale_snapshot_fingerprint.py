import tempfile
import unittest
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

    def test_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as root:
            a = Path(root) / "a.bin"
            a.write_bytes(b"one")
            (Path(root) / "link").symlink_to(a)
            with self.assertRaisesRegex(ValueError, "nonregular"):
                fingerprint(root)


if __name__ == "__main__":
    unittest.main()

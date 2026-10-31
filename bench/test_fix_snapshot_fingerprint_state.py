"""Offline fingerprints must reject metadata changes and fail early on file limits."""
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.snapshot_fingerprint import fingerprint


class FingerprintStateTests(unittest.TestCase):
    def test_file_limit_prevents_reading_next_large_file(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "a.bin").write_bytes(b"a")
            (root / "b.bin").write_bytes(b"b" * 100)
            opened = []
            real_open = os.open

            def observed_open(path, flags, *args, **kwargs):
                opened.append(Path(path).name)
                return real_open(path, flags, *args, **kwargs)

            with patch("bench.scale.snapshot_fingerprint.os.open", side_effect=observed_open):
                with self.assertRaisesRegex(ValueError, "max_files"):
                    fingerprint(root, max_files=1)
            self.assertEqual(opened, ["a.bin"])

    def test_rejects_change_time_only_updates_during_hashing(self):
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "a.bin").write_bytes(b"payload")
            real_fstat = os.fstat
            calls = 0

            def changing_fstat(fd):
                nonlocal calls
                calls += 1
                value = real_fstat(fd)
                if calls != 2:
                    return value
                return SimpleNamespace(
                    st_dev=value.st_dev, st_ino=value.st_ino,
                    st_size=value.st_size, st_mtime_ns=value.st_mtime_ns,
                    st_ctime_ns=value.st_ctime_ns + 1,
                )

            with patch("bench.scale.snapshot_fingerprint.os.fstat", side_effect=changing_fstat):
                with self.assertRaisesRegex(ValueError, "changed during hashing"):
                    fingerprint(folder)


if __name__ == "__main__":
    unittest.main()

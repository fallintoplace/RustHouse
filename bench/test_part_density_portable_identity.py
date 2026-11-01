"""Count metadata must be strict and path-stat time metadata is not portable."""
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale import part_density as module


class PortablePartDensityTests(unittest.TestCase):
    def test_rejects_non_ascii_decimal_cleanly(self):
        with tempfile.TemporaryDirectory() as root:
            part = Path(root) / "part"
            part.mkdir()
            (part / "count.txt").write_bytes("١٢\n".encode("utf-8"))
            with self.assertRaisesRegex(ValueError, "invalid non-ASCII"):
                module.part_density(root)

    def test_metadata_disagreement_between_path_and_fd_does_not_reject_stable_file(self):
        with tempfile.TemporaryDirectory() as root:
            part = Path(root) / "part"
            part.mkdir()
            (part / "count.txt").write_bytes(b"12\n")
            from os import fstat
            calls = 0
            def skew(fd):
                nonlocal calls
                calls += 1
                info = fstat(fd)
                if calls != 1:
                    return info
                return SimpleNamespace(
                    st_mode=info.st_mode, st_dev=info.st_dev, st_ino=info.st_ino,
                    st_size=info.st_size, st_mtime_ns=info.st_mtime_ns,
                    st_ctime_ns=info.st_ctime_ns + 5,
                )
            with patch.object(module.os, "fstat", side_effect=skew):
                self.assertEqual(module.part_density(root)["rows"], 12)


if __name__ == "__main__":
    unittest.main()

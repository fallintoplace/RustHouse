import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

try:
    from tools import offline_backup
except ImportError:
    import offline_backup


class BoundedCopyTests(unittest.TestCase):
    def test_concurrent_append_never_gets_copied_into_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.bin"
            destination = Path(directory) / "copy.bin"
            source.write_bytes(b"original")
            original_fdopen = os.fdopen

            class AppendAfterFirstRead:
                def __init__(self, handle):
                    self.handle = handle
                    self.once = False

                def __enter__(self):
                    return self

                def __exit__(self, *args):
                    return self.handle.__exit__(*args)

                def fileno(self):
                    return self.handle.fileno()

                def read(self, size):
                    block = self.handle.read(size)
                    if not self.once:
                        self.once = True
                        with source.open("ab") as append:
                            append.write(b"x" * 65536)
                    return block

            def fdopen(fd, mode):
                return AppendAfterFirstRead(original_fdopen(fd, mode))

            with patch.object(offline_backup.os, "fdopen", side_effect=fdopen):
                with self.assertRaisesRegex(ValueError, "source grew"):
                    offline_backup.copy_regular_file(source, destination)
            self.assertEqual(destination.read_bytes(), b"original")

    def test_copy_still_preserves_exact_size_and_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.bin"
            destination = Path(directory) / "copy.bin"
            data = bytes(range(256)) * 8193
            source.write_bytes(data)
            self.assertEqual(offline_backup.copy_regular_file(source, destination), destination)
            self.assertEqual(destination.read_bytes(), data)


if __name__ == "__main__":
    unittest.main()

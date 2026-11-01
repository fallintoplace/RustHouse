import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.keyset_export import export


@unittest.skipUnless(os.name == "posix", "directory fsync requires POSIX")
class KeysetPublicationRollbackTests(unittest.TestCase):
    def test_failed_directory_sync_removes_export_and_pending(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "keys.tsv"
            original = os.fsync
            count = 0

            def fail_directory_sync(fd):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("simulated directory sync failure")
                return original(fd)

            with patch("bench.scale.keyset_export.os.fsync", side_effect=fail_directory_sync):
                with self.assertRaisesRegex(OSError, "directory sync failure"):
                    export("unused", "unused", "events", output, 0, 3, page_rows=3,
                           execute=lambda _: b"0\n1\n2\n")
            self.assertEqual(count, 2)
            self.assertFalse(output.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


if __name__ == "__main__":
    unittest.main()

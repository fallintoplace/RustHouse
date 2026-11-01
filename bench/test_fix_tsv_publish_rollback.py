import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.tsv_generator import generate


@unittest.skipUnless(os.name == "posix", "directory fsync requires POSIX")
class TsvPublicationRollbackTests(unittest.TestCase):
    def test_failed_directory_sync_removes_only_newly_published_output(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "ids.tsv"
            original = os.fsync
            count = 0

            def fail_directory_sync(fd):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("simulated directory sync failure")
                return original(fd)

            with patch("bench.scale.tsv_generator.os.fsync", side_effect=fail_directory_sync):
                with self.assertRaisesRegex(OSError, "directory sync failure"):
                    generate(path, 3)
            self.assertEqual(count, 2)
            self.assertFalse(path.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


if __name__ == "__main__":
    unittest.main()

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.permuted_ids import generate


@unittest.skipUnless(os.name == "posix", "directory fsync requires POSIX")
class PermutedPublicationRollbackTests(unittest.TestCase):
    def test_failing_directory_sync_removes_published_output(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "ids.tsv"
            original = os.fsync
            count = 0

            def fail_after_file_sync(fd):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("simulated directory sync failure")
                return original(fd)

            with patch("bench.scale.permuted_ids.os.fsync", side_effect=fail_after_file_sync):
                with self.assertRaisesRegex(OSError, "directory sync failure"):
                    generate(output, 3, seed=17)
            self.assertEqual(count, 2)
            self.assertFalse(output.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


if __name__ == "__main__":
    unittest.main()

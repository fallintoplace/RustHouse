import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.skewed_fixture import generate


@unittest.skipUnless(os.name == "posix", "directory fsync requires POSIX")
class SkewedFixtureRollbackTests(unittest.TestCase):
    def test_final_sync_error_removes_new_fixture(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "skew.tsv"
            original = os.fsync
            count = 0

            def fail_directory_sync(fd):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("simulated directory sync failure")
                return original(fd)

            with patch("bench.scale.skewed_fixture.os.fsync", side_effect=fail_directory_sync):
                with self.assertRaisesRegex(OSError, "directory sync failure"):
                    generate(output, 10, groups=3, hot_percent=50)
            self.assertEqual(count, 2)
            self.assertFalse(output.exists())
            self.assertEqual(list(Path(root).iterdir()), [])


if __name__ == "__main__":
    unittest.main()

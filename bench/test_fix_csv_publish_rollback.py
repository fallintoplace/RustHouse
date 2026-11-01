import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.csv_stream_to_tsv import convert


@unittest.skipUnless(os.name == "posix", "directory fsync requires POSIX")
class CsvPublicationRollbackTests(unittest.TestCase):
    def test_failed_directory_sync_removes_published_tsv(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "source.csv"
            target = Path(root) / "output.tsv"
            source.write_bytes(b"1,hello\n")
            original = os.fsync
            count = 0

            def fail_directory_sync(fd):
                nonlocal count
                count += 1
                if count == 2:
                    raise OSError("simulated directory sync failure")
                return original(fd)

            with patch("bench.scale.csv_stream_to_tsv.os.fsync", side_effect=fail_directory_sync):
                with self.assertRaisesRegex(OSError, "directory sync failure"):
                    convert(source, target)
            self.assertEqual(count, 2)
            self.assertFalse(target.exists())
            self.assertEqual(source.read_bytes(), b"1,hello\n")
            self.assertEqual(list(Path(root).glob("*.pending")), [])


if __name__ == "__main__":
    unittest.main()

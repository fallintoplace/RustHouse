"""Same-size TSV rewrites with restored mtime must fail shard preflight."""
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from bench.scale.tsv_shards import split_tsv


class ChangeTimeShardTests(unittest.TestCase):
    def test_rejects_undetected_source_rewrite_and_cleans_staging(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "source.tsv"
            source.write_bytes(b"0\n1\n")
            destination = root / "shards"
            real_fstat = os.fstat
            calls = 0

            def changed_after_read(fd):
                nonlocal calls
                calls += 1
                info = real_fstat(fd)
                if calls != 2:
                    return info
                return SimpleNamespace(
                    st_mode=info.st_mode, st_dev=info.st_dev, st_ino=info.st_ino,
                    st_size=info.st_size, st_mtime_ns=info.st_mtime_ns,
                    st_ctime_ns=info.st_ctime_ns + 1,
                )

            with patch("bench.scale.tsv_shards.os.fstat", side_effect=changed_after_read):
                with self.assertRaisesRegex(ValueError, "changed while splitting"):
                    split_tsv(source, destination, rows_per_chunk=1)
            self.assertFalse(destination.exists())
            self.assertEqual(list(root.glob(".rusthouse-shards-*")), [])
            self.assertEqual(split_tsv(source, destination, rows_per_chunk=1)["total_rows"], 2)


if __name__ == "__main__":
    unittest.main()

import sys
import tempfile
import unittest
from pathlib import Path
from bench.scale.tsv_shards import split_tsv

class SourceSafetyTests(unittest.TestCase):
    def test_rejects_noninteger_chunk_sizes(self):
        with tempfile.TemporaryDirectory() as tmp:
            input_file = Path(tmp) / "input.tsv"
            input_file.write_bytes(b"1\n")
            for size in (True, 1.5):
                target = Path(tmp) / "output"
                with self.subTest(size=size), self.assertRaisesRegex(ValueError, "rows_per_chunk"):
                    split_tsv(input_file, target, rows_per_chunk=size)
                self.assertFalse(target.exists())

    @unittest.skipIf(sys.platform == "win32", "unprivileged Windows symlinks unavailable")
    def test_rejects_symlinked_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source.tsv"
            source.write_bytes(b"1\n")
            alias = Path(tmp) / "alias.tsv"
            alias.symlink_to(source)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                split_tsv(alias, Path(tmp) / "out")

if __name__ == "__main__":
    unittest.main()

import sys
import tempfile
import unittest
from pathlib import Path

from bench.scale.tsv_profile import profile


class TsvProfileSymlinkTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows CI may disallow symlinks")
    def test_rejects_symlink_to_input(self):
        with tempfile.TemporaryDirectory() as root:
            real = Path(root) / "actual.tsv"
            link = Path(root) / "linked.tsv"
            real.write_bytes(b"1\n2\n")
            link.symlink_to(real)
            with self.assertRaisesRegex(ValueError, "symlinked TSV input"):
                profile(link)
            self.assertEqual(profile(real)["rows"], 2)

    @unittest.skipIf(sys.platform == "win32", "Windows CI may disallow symlinks")
    def test_rejects_dangling_symlink(self):
        with tempfile.TemporaryDirectory() as root:
            link = Path(root) / "dangling.tsv"
            link.symlink_to(Path(root) / "missing.tsv")
            with self.assertRaisesRegex(ValueError, "symlinked TSV input"):
                profile(link)


if __name__ == "__main__":
    unittest.main()

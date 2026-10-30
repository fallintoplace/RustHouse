import sys
import tempfile
import unittest
from pathlib import Path
from bench.scale.tsv_profile import profile

class TsvSourceTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows symlinks require special permissions")
    def test_symlink_input_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "real.tsv"
            path.write_bytes(b"7\n")
            link = Path(tmp) / "alias.tsv"
            link.symlink_to(path)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                profile(link)
            self.assertEqual(profile(path)["rows"], 1)

if __name__ == "__main__":
    unittest.main()

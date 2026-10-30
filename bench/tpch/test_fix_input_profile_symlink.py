import sys
import tempfile
import unittest
from pathlib import Path
from input_profile import profile_table
from schema import names

class TpchInputSymlinkTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows symlinks require special permissions")
    def test_rejects_symlink_to_other_table_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            real = Path(tmp) / "source.tsv"
            real.write_text("\t".join(names("region")) + "\n", encoding="utf-8")
            link = Path(tmp) / "region.tsv"
            link.symlink_to(real)
            with self.assertRaisesRegex(ValueError, "non-symlinked"):
                profile_table(link, "region")
            self.assertEqual(profile_table(real, "region")["rows"], 0)

if __name__ == "__main__":
    unittest.main()

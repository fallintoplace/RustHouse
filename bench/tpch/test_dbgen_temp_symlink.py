import os
import tempfile
import unittest
from pathlib import Path

from convert_dbgen import convert_table


class DbgenUniqueTemporaryFileTests(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "requires POSIX symlink support")
    def test_never_follows_a_preexisting_fixed_temp_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "region.tbl"
            target = root / "region.tsv"
            victim = root / "important.txt"
            old_temp = root / "region.tsv.tmp"
            source.write_bytes(b"1|EUROPE|comment|\n")
            victim.write_bytes(b"must stay unchanged")
            old_temp.symlink_to(victim)
            self.assertEqual(convert_table(source, target, "region"), 1)
            self.assertEqual(victim.read_bytes(), b"must stay unchanged")
            self.assertTrue(old_temp.is_symlink())
            self.assertIn(b"EUROPE", target.read_bytes())


if __name__ == "__main__":
    unittest.main()

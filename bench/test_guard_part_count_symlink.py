import sys
import tempfile
import unittest
from pathlib import Path

from bench.scale.part_density import part_density


class PartCountSymlinkTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "Windows CI does not permit creating symlinks")
    def test_rejects_external_count_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            snapshot = root / "snapshot"
            part = snapshot / "part-0"
            part.mkdir(parents=True)
            outside = root / "outside-count.txt"
            outside.write_text("42\n")
            (part / "count.txt").symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "symlinked count.txt"):
                part_density(snapshot)


if __name__ == "__main__":
    unittest.main()

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from data_manifest import inspect_table
from schema import names


class ManifestPinnedSourceTests(unittest.TestCase):
    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "requires no-follow file opens")
    def test_rejects_symlink_swap_between_stat_and_open(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "region.tsv"
            escaped = root / "outside.tsv"
            data = ("\t".join(names("region")) + "\n1\tEUROPE\tcomment\n").encode()
            path.write_bytes(data)
            escaped.write_bytes(data)
            real_open = os.open

            def swap_source(target, flags, *args, **kwargs):
                if Path(target) == path:
                    path.unlink()
                    path.symlink_to(escaped)
                return real_open(target, flags, *args, **kwargs)

            with patch("data_manifest.os.open", side_effect=swap_source):
                with self.assertRaises(OSError):
                    inspect_table(path, "region")


if __name__ == "__main__":
    unittest.main()

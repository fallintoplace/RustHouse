import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from offline_vacuum import active_label


class ActivePointerReadTests(unittest.TestCase):
    def test_reads_canonical_pointer(self):
        with tempfile.TemporaryDirectory() as root:
            parts = Path(root)
            (parts / ".rusthouse-active").write_bytes(b"g123\n")
            self.assertEqual(active_label(parts), "g123")

    def test_refuses_oversized_pointer_before_consuming_it(self):
        with tempfile.TemporaryDirectory() as root:
            parts = Path(root)
            (parts / ".rusthouse-active").write_bytes(b"g1\n" + b"x" * 1024)
            with self.assertRaisesRegex(ValueError, "invalid active"):
                active_label(parts)

    def test_refuses_inode_replacement_between_stat_and_open(self):
        with tempfile.TemporaryDirectory() as root:
            parts = Path(root)
            pointer = parts / ".rusthouse-active"
            pointer.write_bytes(b"g1\n")
            real_open = os.open

            def switch(path, flags, *args, **kwargs):
                pointer.replace(parts / "retired")
                pointer.write_bytes(b"g2\n")
                return real_open(path, flags, *args, **kwargs)

            with patch("offline_vacuum.os.open", side_effect=switch):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    active_label(parts)


if __name__ == "__main__":
    unittest.main()

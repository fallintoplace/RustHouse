import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale.snapshot_fingerprint import fingerprint


class SnapshotFileBudgetEarlyCheckTests(unittest.TestCase):
    def test_out_of_budget_file_is_never_opened(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            (folder / "a.bin").write_bytes(b"first")
            (folder / "z.bin").write_bytes(b"second")
            real_open = os.open
            opened = []
            def monitored_open(path, flags, *args, **kwargs):
                opened.append(Path(path).name)
                if Path(path).name == "z.bin":
                    raise AssertionError("out-of-budget file must not be opened")
                return real_open(path, flags, *args, **kwargs)
            with patch("bench.scale.snapshot_fingerprint.os.open", side_effect=monitored_open):
                with self.assertRaisesRegex(ValueError, "max_files"):
                    fingerprint(folder, max_files=1)
            self.assertEqual(opened, ["a.bin"])

    def test_exact_file_limit_still_passes(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "one.bin").write_bytes(b"data")
            self.assertEqual(fingerprint(root, max_files=1)["files"], 1)


if __name__ == "__main__":
    unittest.main()

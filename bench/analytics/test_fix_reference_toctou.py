import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "analytics_reference_guard", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class AnalyticsReferenceRaceTests(unittest.TestCase):
    def test_reference_replaced_between_stat_and_open_fails_closed(self):
        with tempfile.TemporaryDirectory() as root:
            expected = Path(root) / "q01.tsv"
            expected.write_bytes(b"original\n")
            actual_open = os.open
            def swap(path, flags, *args, **kwargs):
                expected.replace(Path(root) / "moved.tsv")
                expected.write_bytes(b"original\n")
                return actual_open(path, flags, *args, **kwargs)
            with patch.object(runner.os, "open", side_effect=swap):
                self.assertEqual(runner.reference_failure(expected, b"original\n"),
                                 ("unsafe_reference", str(expected)))

    def test_reference_preserves_exact_byte_matching(self):
        with tempfile.TemporaryDirectory() as root:
            expected = Path(root) / "q01.tsv"
            expected.write_bytes(b"ok\n")
            self.assertIsNone(runner.reference_failure(expected, b"ok\n"))
            self.assertEqual(runner.reference_failure(expected, b"wrong\n")[0],
                             "mismatch")


if __name__ == "__main__":
    unittest.main()

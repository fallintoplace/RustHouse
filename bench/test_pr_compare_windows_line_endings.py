"""Read reports the same way on Windows and Unix."""
import tempfile
import unittest
from pathlib import Path

from bench.pr_bench_compare import _read_bounded_report


class PerformanceLogLineEndingTests(unittest.TestCase):
    def test_converts_crlf_without_changing_text(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.txt"
            path.write_bytes(b"first\r\nsecond\r\n")
            self.assertEqual(_read_bounded_report(path), "first\nsecond\n")

    def test_does_not_change_existing_lf(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.txt"
            path.write_bytes(b"first\nsecond\n")
            self.assertEqual(_read_bounded_report(path), "first\nsecond\n")


if __name__ == "__main__":
    unittest.main()

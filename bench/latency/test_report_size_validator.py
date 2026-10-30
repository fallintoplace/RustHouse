import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from bench.latency import validate_report as target


class ReportSizeTests(unittest.TestCase):
    def test_bounded_latency_report_reads(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "report.json"
            path.write_bytes(b"{}")
            self.assertEqual(target._read_bounded_report(path), "{}")
            with patch.object(target, "MAX_REPORT_BYTES", 2):
                path.write_bytes(b"123")
                with self.assertRaisesRegex(ValueError, "16 MiB"):
                    target._read_bounded_report(path)


if __name__ == "__main__":
    unittest.main()

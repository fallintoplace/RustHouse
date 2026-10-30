import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import bench.release_gate as target


class BoundedReportTests(unittest.TestCase):
    def test_accepts_valid_report_and_rejects_oversize(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "report.json"
            report.write_text("{}", encoding="utf-8")
            self.assertEqual(target._read_bounded_report(report), "{}")
            with patch.object(target, "MAX_REPORT_BYTES", 4):
                report.write_bytes(b"12345")
                with self.assertRaisesRegex(ValueError, "16 MiB"):
                    target._read_bounded_report(report)


if __name__ == "__main__":
    unittest.main()

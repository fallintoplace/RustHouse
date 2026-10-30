import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import bench.pr_parts_compare as target


class PrReportSizeTests(unittest.TestCase):
    def test_small_content_and_oversize_are_distinguished(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "report.txt"
            path.write_text("ok", encoding="utf-8")
            self.assertEqual(target._read_bounded_report(path), "ok")
            with patch.object(target, "MAX_REPORT_BYTES", 2):
                path.write_bytes(b"oversized")
                with self.assertRaisesRegex(ValueError, "16 MiB"):
                    target._read_bounded_report(path)


if __name__ == "__main__":
    unittest.main()

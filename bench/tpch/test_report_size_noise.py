import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import bench.tpch.noise_report as target


class BoundedReportTests(unittest.TestCase):
    def test_valid_report_and_oversize(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "report.json"
            path.write_text("{}", encoding="utf-8")
            self.assertEqual(target._read_bounded_report(path), "{}")
            with patch.object(target, "MAX_REPORT_BYTES", 4):
                path.write_bytes(b"12345")
                with self.assertRaisesRegex(ValueError, "16 MiB"):
                    target._read_bounded_report(path)


if __name__ == "__main__":
    unittest.main()

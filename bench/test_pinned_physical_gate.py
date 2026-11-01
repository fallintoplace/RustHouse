"""Pin a physical report through validation and bounded reading."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.scale import physical_gate


class PinnedReportTests(unittest.TestCase):
    def test_regular_report_still_parses(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_text('{"workload":"sample"}', encoding="utf-8")
            self.assertEqual(physical_gate.parse_report(path)["workload"], "sample")

    def test_swapping_path_before_open_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            replacement = Path(root) / "replacement.json"
            path.write_text('{"workload":"good"}', encoding="utf-8")
            replacement.write_text('{"workload":"swapped"}', encoding="utf-8")
            original_open = os.open
            def swap(*args, **kwargs):
                os.replace(replacement, path)
                return original_open(*args, **kwargs)
            with patch.object(physical_gate.os, "open", side_effect=swap):
                with self.assertRaisesRegex(ValueError, "changed before opening"):
                    physical_gate.parse_report(path)


if __name__ == "__main__":
    unittest.main()

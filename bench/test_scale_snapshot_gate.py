import json
import unittest
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


class SnapshotGateTests(unittest.TestCase):
    def test_accepts_billion_physical_rows(self):
        def run(command, **kwargs):
            self.assertEqual(command[1], "inspect-snapshot")
            return SimpleNamespace(returncode=0, stdout=json.dumps({
                "rows": 1_000_000_001, "parts": 10000,
                "columns": [{"name": "id", "type": "UInt64"}]
            }).encode(), stderr=b"")
        report = inspect_snapshot("rusthouse", "/frozen/parts", run=run)
        self.assertEqual(report["physical_rows"], 1_000_000_001)
        self.assertFalse(report["physical_scan_timed"])

    def test_rejects_small_or_invalid_metadata(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b'{"rows":10,"parts":1,"columns":[1]}', stderr=b"")
        with self.assertRaisesRegex(ValueError, "below required"):
            inspect_snapshot("rusthouse", "parts", run=run)
        with self.assertRaises(ValueError):
            inspect_snapshot("rusthouse", "parts", min_rows=0, run=run)

    def test_rejects_high_row_counts_with_invalid_column_metadata(self):
        for columns in ([1], [{"name": "", "type": "UInt64"}],
                        [{"name": "id", "type": "UInt64"}] * 2):
            def run(command, **kwargs):
                return SimpleNamespace(returncode=0, stdout=json.dumps({
                    "rows": 1_000_000_001, "parts": 10000, "columns": columns
                }).encode(), stderr=b"")
            with self.assertRaisesRegex(ValueError, "columns"):
                inspect_snapshot("rusthouse", "/frozen/parts", run=run)

    def test_failure_propagates(self):
        def run(command, **kwargs):
            return SimpleNamespace(returncode=1, stdout=b"", stderr=b"overlapping")
        with self.assertRaisesRegex(RuntimeError, "overlapping"):
            inspect_snapshot("rusthouse", "parts", run=run)


if __name__ == "__main__":
    unittest.main()

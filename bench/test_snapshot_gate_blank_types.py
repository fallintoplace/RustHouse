"""Physical snapshot release gates must reject blank catalog types."""
import json
import unittest
from types import SimpleNamespace

from bench.scale.snapshot_gate import inspect_snapshot


def mock_snapshot(columns):
    payload = json.dumps({"rows": 1_000_000_000, "parts": 1,
                          "columns": columns}).encode("utf-8")
    return lambda *_a, **_k: SimpleNamespace(
        stdout=payload, stderr=b"", returncode=0)


class SnapshotTypeValidationTests(unittest.TestCase):
    def test_rejects_empty_and_whitespace_only_column_types(self):
        for type_name in ("", " ", "\t", "\r\n", "  \t  "):
            with self.subTest(type_name=type_name):
                with self.assertRaisesRegex(ValueError, "invalid columns"):
                    inspect_snapshot("binary", "part",
                                     run=mock_snapshot([{"name": "id", "type": type_name}]))

    def test_keeps_valid_nested_type_declarations(self):
        report = inspect_snapshot("binary", "part", run=mock_snapshot([
            {"name": "id", "type": "UInt64"},
            {"name": "price", "type": "Nullable(Decimal(18, 4))"},
        ]))
        self.assertEqual(report["physical_rows"], 1_000_000_000)
        self.assertEqual(len(report["columns"]), 2)


if __name__ == "__main__":
    unittest.main()

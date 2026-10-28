import unittest
from unittest.mock import patch

from tools.source_probe import probe


class SourceProbeIdentifierTests(unittest.TestCase):
    @patch("tools.source_probe.read_json")
    def test_rejects_unsafe_identifiers_before_schema_request(self, read):
        read.return_value = {"read_only": True, "source": "database"}
        for name in ("events; DROP TABLE x", "events name", "a-b", "x" * 129,
                     "", "å"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "invalid table"):
                probe("http://127.0.0.1:8123", "database", name)
            self.assertEqual(read.call_count % 1, 0)

    @patch("tools.source_probe.read_json")
    def test_allows_safe_identifier(self, read):
        read.side_effect = [
            {"read_only": True, "source": "database"},
            {"columns": [{"name": "id", "type": "UInt64"}]},
        ]
        self.assertEqual(probe("http://127.0.0.1:8123", "database", "events_2026")["source"],
                         "database")
        self.assertEqual(read.call_count, 2)


if __name__ == "__main__":
    unittest.main()

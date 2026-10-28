import unittest
from unittest.mock import patch
from source_probe import probe, verify_origin

class SourceProbeTests(unittest.TestCase):
    def test_refuses_external_hosts(self):
        with self.assertRaises(ValueError):
            verify_origin("http://example.com:8123")

    @patch("source_probe.read_json")
    def test_detects_source_mismatch(self, read):
        read.return_value = {"source": "generated", "read_only": True}
        with self.assertRaises(ValueError):
            probe("http://127.0.0.1:8123", "database")

    @patch("source_probe.read_json")
    def test_database_schema_required(self, read):
        read.side_effect = [
            {"source": "database", "read_only": True},
            {"columns": [{"name": "id", "type": "UInt64"}]},
        ]
        self.assertEqual(probe("http://127.0.0.1:8123", "database", "events")["source"], "database")


if __name__ == "__main__":
    unittest.main()

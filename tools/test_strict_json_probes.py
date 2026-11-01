"""Probe clients reject non-standard IEEE special values in JSON responses."""
import unittest
from unittest.mock import patch

import cluster_health
import schema_diff
import source_probe


class StrictProbeJsonTests(unittest.TestCase):
    def test_health_rejects_nonfinite_extra_fields(self):
        payload = b'{"total":1,"healthy":1,"peers":[{"address":"node","healthy":true}],"extra":NaN}'
        with patch("cluster_health.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = payload
            with self.assertRaisesRegex(ValueError, "nonfinite JSON"):
                cluster_health.inspect("http://127.0.0.1:8123")

    def test_schema_rejects_nonfinite_extra_fields(self):
        payload = b'{"columns":[{"name":"id","type":"UInt64"}],"extra":Infinity}'
        with patch("schema_diff.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = payload
            with self.assertRaisesRegex(ValueError, "nonfinite JSON"):
                schema_diff.fetch("http://127.0.0.1:8123", "items")

    def test_source_rejects_nonfinite_extra_fields(self):
        payload = b'{"read_only":true,"extra":-Infinity}'
        with patch("source_probe.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = payload
            with self.assertRaisesRegex(ValueError, "nonfinite JSON"):
                source_probe.read_json("http://127.0.0.1:8123/capabilities", 3)


if __name__ == "__main__":
    unittest.main()

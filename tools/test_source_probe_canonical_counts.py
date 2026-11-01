"""Readiness count output must be a canonical unsigned integer."""
import unittest
from unittest.mock import MagicMock, patch

import source_probe


class CanonicalCountTests(unittest.TestCase):
    def test_canonical_counts(self):
        for payload, expected in ((b"rows_checked\n0\n", 0),
                                  (b"rows_checked\n123\n", 123)):
            with self.subTest(payload=payload):
                response = MagicMock()
                response.__enter__.return_value.read.return_value = payload
                with patch.object(source_probe.NO_REDIRECT_OPENER, "open", return_value=response):
                    self.assertEqual(source_probe.scan_rows("http://127.0.0.1:8123",
                                                            "events", 5), expected)

    def test_rejects_padded_or_out_of_range_counts(self):
        for payload in (b"rows_checked\n0001\n", b"rows_checked\n00\n",
                        b"rows_checked\n18446744073709551616\n",
                        b"rows_checked\n+1\n"):
            with self.subTest(payload=payload):
                response = MagicMock()
                response.__enter__.return_value.read.return_value = payload
                with patch.object(source_probe.NO_REDIRECT_OPENER, "open", return_value=response):
                    with self.assertRaises(ValueError):
                        source_probe.scan_rows("http://127.0.0.1:8123", "events", 5)


if __name__ == "__main__":
    unittest.main()

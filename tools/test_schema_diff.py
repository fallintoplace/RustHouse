import unittest
from schema_diff import columns, validate_url

class SchemaTests(unittest.TestCase):
    def test_order_and_types_are_retained(self):
        self.assertEqual(columns({"columns": [{"name": "id", "type": "UInt64"},
                                               {"name": "s", "type": "String"}]}),
                         [("id", "UInt64"), ("s", "String")])
    def test_invalid_schema(self):
        with self.assertRaises(ValueError):
            columns({"columns": [{"name": "id"}]})
    def test_restricts_external_network(self):
        with self.assertRaises(ValueError):
            validate_url("http://example.org:8123")


    def test_redirect_handler_refuses_non_loopback_targets(self):
        from schema_diff import NoRedirect
        self.assertIsNone(NoRedirect().redirect_request(None, None, 302, "Found", {}, "http://example.org/"))

    def test_schema_fetch_uses_redirect_rejecting_opener(self):
        from unittest.mock import patch
        from schema_diff import fetch
        with patch("schema_diff.NO_REDIRECT_OPENER") as opener:
            opener.open.return_value.__enter__.return_value.read.return_value = b'{"columns":[{"name":"id","type":"UInt64"}]}'
            self.assertEqual(fetch("http://127.0.0.1:8123", "items", 4), [("id", "UInt64")])
            opener.open.assert_called_once_with("http://127.0.0.1:8123/schema?table=items", timeout=4)

if __name__ == "__main__":
    unittest.main()

import unittest
from schema_diff import columns, validate_url, compatible_columns

class SchemaTests(unittest.TestCase):
    def test_order_and_types_are_retained(self):
        self.assertEqual(columns({"columns": [{"name": "id", "type": "UInt64"},
                                               {"name": "s", "type": "String"}]}),
                         [("id", "UInt64"), ("s", "String")])
    def test_append_only_compatibility_is_opt_in(self):
        old = [("id", "UInt64"), ("label", "String")]
        new = old + [("region", "String")]
        self.assertFalse(compatible_columns(old, new))
        self.assertTrue(compatible_columns(old, new, allow_additive=True))
        self.assertFalse(compatible_columns(new, old, allow_additive=True))
        self.assertFalse(compatible_columns(old, [("id", "String"), old[1]],
                                            allow_additive=True))
        self.assertFalse(compatible_columns(old, [old[1], old[0]],
                                            allow_additive=True))

    def test_invalid_schema(self):
        with self.assertRaises(ValueError):
            columns({"columns": [{"name": "id"}]})
    def test_empty_schema_cannot_be_reported_as_compatible(self):
        with self.assertRaisesRegex(ValueError, "columns"):
            columns({"columns": []})
        with self.assertRaisesRegex(ValueError, "columns"):
            columns({"columns": None})

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

    def test_rejects_whitespace_only_schema_fields(self):
        for malformed in (
            {"name": " ", "type": "UInt64"},
            {"name": "id", "type": "  "},
        ):
            with self.subTest(malformed=malformed), self.assertRaises(ValueError):
                columns({"columns": [malformed]})

if __name__ == "__main__":
    unittest.main()

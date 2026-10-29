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


    def test_rejects_whitespace_only_schema_fields(self):
        for malformed in (
            {"name": " ", "type": "UInt64"},
            {"name": "id", "type": "  "},
        ):
            with self.subTest(malformed=malformed), self.assertRaises(ValueError):
                columns({"columns": [malformed]})


if __name__ == "__main__":
    unittest.main()

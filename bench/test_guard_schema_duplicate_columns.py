import unittest

from tools.schema_diff import columns


class SchemaIntegrityTests(unittest.TestCase):
    def test_rejects_duplicate_column_names(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            columns({"columns": [{"name": "id", "type": "UInt64"},
                                 {"name": "id", "type": "String"}]})

    def test_rejects_empty_column_metadata(self):
        for column in ({"name": "", "type": "UInt64"}, {"name": "x", "type": ""}):
            with self.subTest(column=column), self.assertRaisesRegex(ValueError, "empty"):
                columns({"columns": [column]})

    def test_rejects_nonobject_payload(self):
        with self.assertRaisesRegex(ValueError, "object"):
            columns([])

    def test_preserves_order_of_valid_columns(self):
        self.assertEqual(columns({"columns": [{"name": "a", "type": "UInt8"},
                                              {"name": "b", "type": "String"}]}),
                         [("a", "UInt8"), ("b", "String")])


if __name__ == "__main__":
    unittest.main()

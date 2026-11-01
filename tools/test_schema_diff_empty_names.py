"""Missing column names need actionable schema diagnostics."""
import unittest

from schema_diff import columns


class EmptySchemaColumnTests(unittest.TestCase):
    def test_empty_name_is_rejected_distinctly(self):
        with self.assertRaisesRegex(ValueError, "empty schema column name"):
            columns({"columns": [{"name": "", "type": "UInt64"}]})

    def test_other_invalid_identifier_remains_invalid(self):
        with self.assertRaisesRegex(ValueError, "invalid schema column identifier"):
            columns({"columns": [{"name": "bad-name", "type": "UInt64"}]})


if __name__ == "__main__":
    unittest.main()

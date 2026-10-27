import unittest
from schema import TABLES, create_sql, explain_scaling, names


class SchemaTests(unittest.TestCase):
    def test_eight_canonical_tables_and_field_counts(self):
        self.assertEqual(len(TABLES), 8)
        expected = {
            "region": 3, "nation": 4, "supplier": 7, "customer": 8,
            "part": 9, "partsupp": 5, "orders": 9, "lineitem": 16,
        }
        self.assertEqual({table: len(fields) for table, fields in TABLES.items()}, expected)
        for table, fields in TABLES.items():
            self.assertEqual(len(set(names(table))), len(fields))
            self.assertTrue(create_sql(table).startswith("CREATE TABLE " + table))
            self.assertTrue(all(kind in ("UInt64", "Int64", "Date", "String")
                                for _, kind, _ in fields))

    def test_fixed_point_is_explicit(self):
        lineitem = {name: scale for name, _, scale in TABLES["lineitem"]}
        self.assertEqual(lineitem["l_discount"], 10000)
        self.assertEqual(lineitem["l_extendedprice"], 100)
        self.assertEqual(lineitem["l_quantity"], 100)
        self.assertEqual(lineitem["l_shipdate"], 1)
        self.assertFalse(explain_scaling()["certified"])


if __name__ == "__main__":
    unittest.main()

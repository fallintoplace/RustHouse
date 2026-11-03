import unittest
from pathlib import Path
from query_feature_inventory import features, inventory


class CanonicalTpchFeaturesTests(unittest.TestCase):
    def test_tracks_typed_expression_shapes(self):
        source = Path(__file__).resolve().parent / "queries"
        by_query = {row["query"]: set(row["features"])
                    for row in inventory(source)["results"]}
        self.assertIn("arithmetic_aggregate", by_query[1])
        self.assertIn("arithmetic_aggregate", by_query[3])
        self.assertIn("conditional_aggregate", by_query[12])
        self.assertIn("aggregate_ratio", by_query[8])
        self.assertIn("decimal_literal", by_query[14])
        self.assertIn("decimal_literal", by_query[17])

    def test_ignores_embedded_sql_inside_quotes(self):
        sql = "SELECT 'sum(a * b) / SUM(x) 100.00' AS label FROM source"
        for name in ("arithmetic_aggregate", "aggregate_ratio", "decimal_literal"):
            self.assertNotIn(name, features(sql))

    def test_simple_aggregate_is_not_marked_as_arithmetic(self):
        self.assertNotIn("arithmetic_aggregate", features("SELECT sum(amount) FROM payments"))


if __name__ == "__main__":
    unittest.main()

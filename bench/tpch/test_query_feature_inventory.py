import tempfile
import unittest
from pathlib import Path
from query_feature_inventory import features, inventory, mask_literals


class FeatureInventoryTests(unittest.TestCase):
    def test_ignores_keyword_text_inside_literals_and_comments(self):
        sql = ("SELECT '-- EXISTS (SELECT 1)' AS label /* GROUP BY z */ "
               "FROM t WHERE x LIKE '%CASE WHEN%'")
        self.assertEqual(features(sql), ["like"])

    def test_classifies_tpch_specific_structures(self):
        sql = ("WITH revenue AS (SELECT sum(amount) FROM sales) "
               "SELECT substring(phone FROM 1 FOR 2) FROM customer, orders "
               "WHERE NOT EXISTS (SELECT 1 FROM orders) "
               "AND id IN (SELECT id FROM flags) GROUP BY id HAVING count(*) > 0")
        tags = features(sql)
        for expected in ("with_cte", "nested_select", "substring", "comma_join",
                         "not_exists", "exists", "in_subquery", "group_by", "having"):
            self.assertIn(expected, tags)

    def test_handles_escaped_quote_characters(self):
        self.assertEqual(mask_literals("SELECT 'EXISTS(''SELECT'')' FROM a"),
                         "SELECT " + " " * len("'EXISTS(''SELECT'')'") + " FROM a")
        with self.assertRaisesRegex(ValueError, "unterminated"):
            features("SELECT 'not terminated")

    def test_requires_all_twenty_two_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for number in range(1, 23):
                (root / f"q{number:02d}.sql").write_text(
                    "SELECT count(*) FROM trips")
            result = inventory(root)
            self.assertEqual(result["total_queries"], 22)
            self.assertEqual(len(result["results"]), 22)
            self.assertFalse(result["official_tpch_score"])
            (root / "q22.sql").unlink()
            with self.assertRaises(FileNotFoundError):
                inventory(root)


if __name__ == "__main__":
    unittest.main()

import unittest
from feature_gaps import analyze, classify

class StatusTypesTests(unittest.TestCase):
    def test_unhashable_status_values_are_classified(self):
        for value in ([], {}, None, 12):
            with self.subTest(value=value):
                self.assertEqual(classify({"status": value}), "unexpected_status")

    def test_report_keeps_query_count_for_malformed_status(self):
        report = analyze({"results": [{"query": 1, "status": []}]})
        self.assertEqual(report["queries_examined"], 1)
        self.assertEqual(report["categories"][0]["category"], "unexpected_status")

if __name__ == "__main__":
    unittest.main()

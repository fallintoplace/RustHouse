import unittest

from feature_gaps import analyze, classify


class FirstStopDiagnosticsTests(unittest.TestCase):
    def test_classifies_known_canonical_tpch_errors(self):
        cases = {
            "grouped_aggregate_expression": "expected ')' after grouped aggregate",
            "statement_suffix": "unexpected tokens after SELECT statement",
            "identifier_or_expression": "expected an identifier",
            "grouped_aggregate_shape": "grouped aggregates require GROUP BY",
            "source_or_projection": "a table source is required",
            "comparison_predicate": "expected a comparison operator",
            "conditional_aggregate": "grouped CASE requires THEN",
            "join_syntax": "JOIN columns must be qualified (alias.column)",
            "constant_numeric": "SELECT literals must be signed 64-bit integers",
            "statement_type": "only SELECT statements are supported",
        }
        for category, message in cases.items():
            with self.subTest(message=message):
                self.assertEqual(classify({"status": "error", "error": message}), category)

    def test_classification_does_not_upgrade_execution_coverage(self):
        report = {"results": [
            {"query": n, "status": "error", "error": "expected ')' after grouped aggregate"}
            for n in range(1, 23)
        ]}
        result = analyze(report, require_all_22=True)
        self.assertEqual(result["verified_correct_percent"], 0)
        self.assertEqual(result["execution_success_percent"], 0)
        self.assertEqual(result["categories"][0], {
            "category": "grouped_aggregate_expression",
            "count": 22,
            "queries": list(range(1, 23)),
        })


if __name__ == "__main__":
    unittest.main()

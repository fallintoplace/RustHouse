import tempfile
import unittest
from pathlib import Path
from feature_gaps import analyze, classify, load_report


class FeatureGapTests(unittest.TestCase):
    def test_grouping_labels_and_verified_coverage(self):
        report = {"results": [
            {"query": 1, "status": "passed"},
            {"query": 2, "status": "unverified"},
            {"query": 3, "status": "error",
             "error": "JOIN supports at most 16384 input rows per table"},
            {"query": 4, "status": "error", "error": "unsupported scalar operator"},
            {"query": 5, "status": "mismatch"},
        ]}
        result = analyze(report)
        self.assertEqual(result["execution_success_percent"], 40)
        self.assertEqual(result["verified_correct_percent"], 20)
        categories = {entry["category"]: entry["queries"] for entry in result["categories"]}
        self.assertEqual(categories["join_capacity"], [3])
        self.assertEqual(categories["unimplemented_operator"], [4])
        self.assertEqual(categories["mismatch"], [5])
        self.assertFalse(result["official_tpch_score"])

    def test_unknown_failures_are_not_guessed(self):
        self.assertEqual(classify({"status": "error", "error": "failed for unknown reason"}),
                         "unclassified_error")
        self.assertEqual(classify({"status": "timeout", "error": "unrelated"}), "timeout")

    def test_resource_capacity_failures_are_not_generic_sql_unsupported(self):
        for message in (
            "GROUP BY spill exceeds 32 GiB budget",
            "query exceeds 16384 parts",
            "too many retained rows",
        ):
            self.assertEqual(
                classify({"status": "error", "error": message}),
                "resource_limit",
            )
        with self.assertRaisesRegex(ValueError, "object"):
            analyze(["not a report"])

    def test_all_22_validation_and_duplicate_rejection(self):
        with self.assertRaisesRegex(ValueError, "each query ID"):
            analyze({"results": [{"query": 1, "status": "passed"}]}, require_all_22=True)
        with self.assertRaisesRegex(ValueError, "unique"):
            analyze({"results": [{"query": 1}, {"query": 1}]})
        with self.assertRaisesRegex(ValueError, "array"):
            analyze({})


class InvalidGapStatusTests(unittest.TestCase):
    def test_malformed_status_does_not_crash_or_claim_success(self):
        for status in ([], {}, True, 0):
            with self.subTest(status=status):
                self.assertEqual(classify({"status": status}), "unexpected_status")
                with self.assertRaisesRegex(ValueError, "status"):
                    analyze({"results": [{"query": 1, "status": status}]})



class BoundedFeatureGapReportTests(unittest.TestCase):
    def test_rejects_oversized_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_bytes(b" " * (2 * 1024 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "exceeds 2 MiB"):
                load_report(path)

    def test_loads_normal_report(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_bytes(b'{"results":[{"query":1,"status":"passed"}]}')
            self.assertEqual(analyze(load_report(path))["verified_correct_percent"], 100)

if __name__ == "__main__":
    unittest.main()

import unittest

from coverage_delta import compare


def report():
    return {"results": [
        {"query": n, "status": "error", "error": "expected ')' after grouped aggregate"}
        for n in range(1, 23)
    ]}


class ErrorTransitionCoverageTests(unittest.TestCase):
    def test_reports_new_first_error_without_claiming_sql_progress(self):
        before = report()
        after = report()
        after["results"][0]["error"] = "unexpected tokens after SELECT statement"
        after["results"][1]["status"] = "passed"
        result = compare(before, after)
        self.assertEqual(result["error_transition_count"], 1)
        self.assertEqual(result["error_transitions"], [{
            "query": 1,
            "before_error": "expected ')' after grouped aggregate",
            "after_error": "unexpected tokens after SELECT statement",
        }])
        self.assertEqual(result["counts"]["improved"], 1)
        self.assertEqual(result["verified_after"], 1)
        self.assertEqual(result["counts"]["unchanged"], 21)

    def test_ignores_missing_error_details(self):
        older = report()
        newer = report()
        newer["results"][0].pop("error")
        self.assertEqual(compare(older, newer)["error_transition_count"], 0)


if __name__ == "__main__":
    unittest.main()

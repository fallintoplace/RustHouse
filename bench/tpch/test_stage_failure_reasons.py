import unittest
from stage_coverage import combine


def report(status):
    return {"results": [{"query": n, "status": status} for n in range(1, 23)]}


class StageDiagnosticsTests(unittest.TestCase):
    def test_keeps_both_failure_reasons_when_only_one_stage_failed(self):
        parser = report("parse_error")
        execution = report("error")
        parser["results"][0]["error"] = "expected ')' after grouped aggregate"
        execution["results"][0]["error"] = "unexpected tokens after SELECT statement"
        parser["results"][1]["status"] = "parsed"
        execution["results"][1]["status"] = "passed"
        result = combine(parser, execution)
        self.assertEqual(result["results"][0]["parse_error"],
                         "expected ')' after grouped aggregate")
        self.assertEqual(result["results"][0]["execution_error"],
                         "unexpected tokens after SELECT statement")
        self.assertEqual(result["results"][0]["stage"], "not_parsed")
        self.assertNotIn("parse_error", result["results"][1])
        self.assertNotIn("execution_error", result["results"][1])
        self.assertEqual(result["categories"]["verified"], 1)

    def test_limits_error_messages(self):
        parser = report("parse_error")
        execution = report("error")
        parser["results"][0]["error"] = "p" * 600
        execution["results"][0]["error"] = "e" * 600
        row = combine(parser, execution)["results"][0]
        self.assertEqual(len(row["parse_error"]), 500)
        self.assertEqual(len(row["execution_error"]), 500)


if __name__ == "__main__":
    unittest.main()

import unittest
from parser_coverage_delta import compare


def report(status):
    return {"results": [{"query": n, "status": status} for n in range(1, 23)]}


class ParserCoverageDeltaTests(unittest.TestCase):
    def test_unblocks_and_regresses_independently_of_execution(self):
        before = report("parse_error")
        after = report("parse_error")
        before["results"][1]["status"] = "parsed"
        after["results"][0]["status"] = "parsed"
        after["results"][1]["status"] = "invalid_query"
        before["results"][2]["error"] = "expected an identifier"
        after["results"][2]["error"] = "expected ')' after grouped aggregate"
        outcome = compare(before, after)
        self.assertEqual(outcome["unblocked_queries"], [1])
        self.assertEqual(outcome["regressed_queries"], [2])
        self.assertEqual(outcome["parsed_after"], 1)
        self.assertEqual(outcome["first_error_transitions"], [{
            "query": 3, "before_error": "expected an identifier",
            "after_error": "expected ')' after grouped aggregate",
        }])
        self.assertFalse(outcome["official_tpch_score"])

    def test_requires_matching_sql_when_fingerprints_are_available(self):
        older, newer = report("parse_error"), report("parsed")
        older["query_sha256"] = "a" * 64
        newer["query_sha256"] = "b" * 64
        with self.assertRaisesRegex(ValueError, "fingerprints"):
            compare(older, newer)
        newer.pop("query_sha256")
        with self.assertRaisesRegex(ValueError, "fingerprints"):
            compare(older, newer)
        newer["query_sha256"] = older["query_sha256"]
        self.assertEqual(compare(older, newer)["parsed_after"], 22)

    def test_rejects_missing_duplicate_and_invalid_status(self):
        older = report("parsed")
        newer = report("parsed")
        newer["results"].pop()
        with self.assertRaisesRegex(ValueError, "all 22"):
            compare(older, newer)
        newer = report("parsed")
        newer["results"][0]["query"] = True
        with self.assertRaisesRegex(ValueError, "distinct"):
            compare(older, newer)
        newer = report("parsed")
        newer["results"][0]["status"] = []
        with self.assertRaisesRegex(ValueError, "status"):
            compare(older, newer)


if __name__ == "__main__":
    unittest.main()

import unittest
from stage_coverage import combine


def report(status):
    return {"results": [{"query": i, "status": status} for i in range(1, 23)]}


class StageCoverageTests(unittest.TestCase):
    def test_all_verified(self):
        result = combine(report("parsed"), report("passed"))
        self.assertEqual(result["categories"]["verified"], 22)
        self.assertFalse(result["official_tpch_score"])

    def test_stages_are_not_interchangeable(self):
        parser = report("parsed")
        execution = report("passed")
        parser["results"][0]["status"] = "parse_error"
        execution["results"][1]["status"] = "unverified"
        execution["results"][2]["status"] = "error"
        result = combine(parser, execution)
        self.assertEqual(result["categories"], {
            "not_parsed": 0, "execution_failed": 1, "unverified": 1,
            "verified": 19, "inconsistent": 1,
        })

    def test_rejects_missing_duplicate_and_boolean_ids(self):
        for bad in (report("parsed")["results"][:-1],
                    report("parsed")["results"][:-1] +
                    [{"query": 1, "status": "parsed"}],
                    [{"query": True, "status": "parsed"}] +
                    report("parsed")["results"][1:]):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    combine({"results": bad}, report("passed"))

    def test_rejects_unrecognized_stages(self):
        parser = report("parsed")
        parser["results"][0]["status"] = "passed"
        with self.assertRaisesRegex(ValueError, "status"):
            combine(parser, report("passed"))


if __name__ == "__main__":
    unittest.main()

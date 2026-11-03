import unittest
from coverage_delta import compare


def report(status):
    return {"results": [{"query": n, "status": status} for n in range(1, 23)]}


class CoverageDeltaTests(unittest.TestCase):
    def test_verified_improvements_and_regressions(self):
        before = report("error")
        after = report("error")
        before["results"][0]["status"] = "passed"
        after["results"][0]["status"] = "unverified"
        after["results"][1]["status"] = "passed"
        after["results"][2]["status"] = "unverified"
        after["results"][3]["status"] = "mismatch"
        comparison = compare(before, after)
        self.assertEqual(comparison["counts"], {
            "improved": 2, "regressed": 1, "changed": 1, "unchanged": 18,
        })
        self.assertEqual(comparison["verified_before"], 1)
        self.assertEqual(comparison["verified_after"], 1)
        self.assertFalse(comparison["official_tpch_score"])

    def test_identical_reports_have_no_changes(self):
        result = compare(report("passed"), report("passed"))
        self.assertEqual(result["changes"], [])
        self.assertEqual(result["counts"]["unchanged"], 22)

    def test_invalid_ids_cannot_fake_improvements(self):
        before = report("passed")
        for value in (True, 1.0, 0, 23, 2):
            with self.subTest(value=value):
                bad = report("passed")
                bad["results"][0]["query"] = value
                with self.assertRaises(ValueError):
                    compare(before, bad)

    def test_incomplete_reports_are_rejected(self):
        partial = {"results": [{"query": 1, "status": "passed"}]}
        with self.assertRaisesRegex(ValueError, "all 22"):
            compare(partial, report("passed"))


if __name__ == "__main__":
    unittest.main()

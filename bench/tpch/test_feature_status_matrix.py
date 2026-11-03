import unittest
from feature_status_matrix import summarize


def features():
    return {"results": [
        {"query": n, "features": ["case", "group_by"] if n % 2 else ["group_by"]}
        for n in range(1, 23)
    ]}


def execution():
    return {"results": [
        {"query": n, "status": "passed" if n == 1 else "unverified" if n == 2 else "error"}
        for n in range(1, 23)
    ]}


class FeatureStatusMatrixTests(unittest.TestCase):
    def test_does_not_equate_unverified_with_correctness(self):
        result = summarize(features(), execution())
        groups = {record["feature"]: record for record in result["features"]}
        self.assertEqual(groups["group_by"]["query_count"], 22)
        self.assertEqual(groups["group_by"]["verified_count"], 1)
        self.assertEqual(groups["group_by"]["unverified_count"], 1)
        self.assertEqual(groups["group_by"]["failed_count"], 20)
        self.assertEqual(groups["case"]["verified_count"], 1)
        self.assertEqual(groups["case"]["failed_count"], 10)
        self.assertFalse(result["feature_support_inferred"])
        self.assertFalse(result["official_tpch_score"])

    def test_refuses_incomplete_or_duplicate_reports(self):
        f, e = features(), execution()
        e["results"] = e["results"][:-1]
        with self.assertRaisesRegex(ValueError, "all 22"):
            summarize(f, e)
        e = execution()
        f["results"][1]["query"] = 1
        with self.assertRaisesRegex(ValueError, "distinct"):
            summarize(f, e)

    def test_rejects_duplicate_feature_names_and_invalid_statuses(self):
        f, e = features(), execution()
        f["results"][0]["features"] = ["case", "case"]
        with self.assertRaisesRegex(ValueError, "unique"):
            summarize(f, e)
        f = features()
        e["results"][0]["status"] = "parsed"
        with self.assertRaisesRegex(ValueError, "status"):
            summarize(f, e)


if __name__ == "__main__":
    unittest.main()

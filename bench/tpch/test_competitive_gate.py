import copy
import unittest

from bench.tpch.competitive_gate import summarize


def verified_report():
    sample = {
        "query": 1,
        "status": "passed",
        "reference_sha256": "b" * 64,
        "rusthouse_samples_seconds": [1.0, 1.0],
        "clickhouse_samples_seconds": [2.0, 2.0],
        "rusthouse_median_seconds": 1.0,
        "clickhouse_median_seconds": 2.0,
        "rusthouse_p90_seconds": 1.0,
        "clickhouse_p90_seconds": 2.0,
        "clickhouse_over_rusthouse": 2.0,
        "clickhouse_p90_over_rusthouse": 2.0,
    }
    return {
        "workload": "paired-clickhouse-cli-sql-latency",
        "official_tpch_score": False,
        "all_queries_verified": True,
        "comparison": "exact",
        "query_sha256": "a" * 64,
        "runs": 1,
        "warmups": 0,
        "results": [sample],
        "speedup_geomean_clickhouse_over_rusthouse": 2.0,
    }


class CompetitiveGateTests(unittest.TestCase):
    def test_accepts_complete_verified_raw_samples_and_supports_a_target(self):
        report = summarize(verified_report(), min_queries=1, min_geomean=1.5)
        self.assertTrue(report["reference_verified"])
        self.assertTrue(report["meets_speedup_target"])
        self.assertEqual(report["geomean_clickhouse_over_rusthouse"], 2.0)
        self.assertEqual(report["worst_p90_clickhouse_over_rusthouse"], 2.0)
        self.assertFalse(summarize(verified_report(), min_geomean=3)["meets_speedup_target"])

    def test_rejects_missing_coverage_or_duplicate_queries(self):
        with self.assertRaisesRegex(ValueError, "coverage"):
            summarize(verified_report(), min_queries=22)
        report = verified_report()
        report["results"].append(copy.deepcopy(report["results"][0]))
        with self.assertRaisesRegex(ValueError, "unique"):
            summarize(report)

    def test_rejects_tampered_aggregates_unverified_or_bad_references(self):
        for change, message in [
            (lambda report: report.update(all_queries_verified=False), "verified"),
            (lambda report: report.update(speedup_geomean_clickhouse_over_rusthouse=4), "aggregate"),
            (lambda report: report["results"][0].update(rusthouse_median_seconds=2), "median"),
            (lambda report: report["results"][0].update(clickhouse_p90_over_rusthouse=3), "tail ratio"),
            (lambda report: report["results"][0].update(reference_sha256="wrong"), "reference"),
            (lambda report: report["results"][0].update(status="unverified"), "verified"),
            (lambda report: report["results"][0].update(
                rusthouse_samples_seconds=[1.0, float("nan")]
            ), "samples"),
        ]:
            report = verified_report()
            change(report)
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                summarize(report)

    def test_rejects_disguised_official_benchmarks_and_invalid_targets(self):
        report = verified_report()
        report["official_tpch_score"] = True
        with self.assertRaisesRegex(ValueError, "official"):
            summarize(report)
        with self.assertRaisesRegex(ValueError, "min geomean"):
            summarize(verified_report(), min_geomean=float("inf"))


if __name__ == "__main__":
    unittest.main()

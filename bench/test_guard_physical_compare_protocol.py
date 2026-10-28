import unittest

from bench.scale.compare_physical import compare


def report(**kw):
    data = {"workload": "rusthouse-persisted-wide-part-full-scans",
            "rows_written_and_verified": 100,
            "runs": 5, "warmups": 2,
            "platform": "Linux", "cpu_count": 4,
            "source": "persisted immutable wide parts",
            "note": "fresh CLI processes",
            "count_even": {"query": "count", "expected": 50,
                           "median_seconds": 1.0, "verified": True},
            "sum_even": {"query": "sum", "expected": 2450,
                         "median_seconds": 1.0, "verified": True}}
    data.update(kw)
    return data


class PhysicalProtocolTests(unittest.TestCase):
    def test_rejects_mixed_benchmark_protocols(self):
        baseline = report()
        for field, value in (("runs", 9), ("warmups", 0), ("platform", "Windows"),
                             ("cpu_count", 8), ("source", "generated only"),
                             ("note", "cached in-process")):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, field):
                compare(baseline, report(**{field: value}))

    def test_rejects_missing_metadata_on_only_one_side(self):
        baseline = report()
        candidate = report()
        del candidate["runs"]
        with self.assertRaisesRegex(ValueError, "runs"):
            compare(baseline, candidate)

    def test_accepts_matching_protocol(self):
        self.assertTrue(compare(report(), report())["all_within_budget"])


if __name__ == "__main__":
    unittest.main()

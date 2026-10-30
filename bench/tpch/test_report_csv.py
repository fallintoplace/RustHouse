import csv
import io
import unittest

from report_csv import rows_from_report, write_csv


class ReportCSVTests(unittest.TestCase):
    def test_exports_verified_timings_without_fake_failure_numbers(self):
        report = {"results": [
            {"query": 6, "status": "passed",
             "timings_seconds": [0.01, 0.02],
             "latency": {"median_seconds": 0.015, "p90_seconds": 0.02},
             "error": None},
            {"query": 12, "status": "timeout",
             "timings_seconds": [0.01],
             "latency": {"median_seconds": 0.01, "p90_seconds": 0.01},
             "error": "slow"},
        ]}
        output = io.StringIO()
        write_csv(report, output)
        rows = list(csv.DictReader(io.StringIO(output.getvalue())))
        self.assertEqual(rows[0]["median_ms"], "15.000000")
        self.assertEqual(rows[0]["samples"], "2")
        self.assertEqual(rows[1]["median_ms"], "")
        self.assertEqual(rows[1]["samples"], "0")
        self.assertEqual(rows[1]["error"], "slow")

    def test_rejects_bad_or_repeated_query_ids(self):
        for rows in ([], [{"query": 0, "status": "timeout"}],
                     [{"query": True, "status": "timeout"}],
                     [{"query": 1, "status": "timeout"},
                      {"query": 1, "status": "timeout"}]):
            with self.subTest(rows=rows):
                with self.assertRaises(ValueError):
                    rows_from_report({"results": rows})

    def test_rejects_invalid_successful_measurements(self):
        for values, latency in [
            ([], None),
            ([0], {"median_seconds": 0, "p90_seconds": 0}),
            ([float("nan")], {"median_seconds": 1, "p90_seconds": 1}),
            ([1], {"median_seconds": 1}),
            ([True], {"median_seconds": 1, "p90_seconds": 1}),
        ]:
            with self.subTest(values=values, latency=latency):
                with self.assertRaisesRegex(ValueError, "invalid"):
                    rows_from_report({"results": [{
                        "query": 1, "status": "passed",
                        "timings_seconds": values, "latency": latency,
                    }]})

    def test_rejects_incorrect_successful_latency_summary(self):
        payload = {"results": [{"query": 1, "status": "passed",
                   "timings_seconds": [1.0, 2.0, 3.0],
                   "latency": {"median_seconds": 1.0, "p90_seconds": 3.0}}]}
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            rows_from_report(payload)
        payload["results"][0]["latency"] = {
            "median_seconds": 2.0, "p90_seconds": 3.0}
        self.assertEqual(rows_from_report(payload)[0]["median_ms"], "2000.000000")

    def test_rejects_incomplete_json(self):
        for payload in ({}, {"results": None}, {"results": [{}]}):
            with self.assertRaises(ValueError):
                rows_from_report(payload)


class ReportCSVStatusTests(unittest.TestCase):
    def test_rejects_unknown_or_non_string_status(self):
        for status in ("passsed", None, [], True):
            with self.subTest(status=status):
                with self.assertRaisesRegex(ValueError, "unknown benchmark"):
                    rows_from_report({"results": [{"query": 1, "status": status}]})

if __name__ == "__main__":
    unittest.main()

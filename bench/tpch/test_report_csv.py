import csv
import io
import unittest

from report_csv import rows_from_report, write_csv


class ReportCSVTests(unittest.TestCase):
    def test_rejects_oversized_csv_source_report(self):
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from report_csv import load_report
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "report.json"
            source.write_bytes(b"0" * 17)
            with patch("report_csv.MAX_REPORT_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    load_report(source)


    def test_rejects_report_replaced_between_stat_and_open(self):
        import os
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from report_csv import load_report

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            alternate = Path(root) / "alternate.json"
            path.write_text('{"results":[]}', encoding="utf-8")
            alternate.write_text('{"results":[1]}', encoding="utf-8")
            original_open = os.open

            def swap(filename, flags):
                path.unlink()
                alternate.rename(path)
                return original_open(filename, flags)

            with patch("report_csv.os.open", side_effect=swap):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    load_report(path)

    def test_failed_csv_export_does_not_truncate_existing_file(self):
        import json
        import tempfile
        from pathlib import Path
        from report_csv import main

        with tempfile.TemporaryDirectory() as root:
            input_path = Path(root) / "report.json"
            output_path = Path(root) / "summary.csv"
            output_path.write_text("old-complete-report\\n", encoding="utf-8")
            input_path.write_text(json.dumps({"results": [{"query": 1,
                "status": "passed", "timings_seconds": [0]}]}), encoding="utf-8")
            with self.assertRaises(SystemExit):
                main(["--input", str(input_path), "--output", str(output_path)])
            self.assertEqual(output_path.read_text(), "old-complete-report\\n")
            self.assertFalse(list(output_path.parent.glob(".summary.csv.*.tmp")))

    def test_csv_output_cannot_overwrite_source_json(self):
        import json
        import tempfile
        from pathlib import Path
        from report_csv import main

        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "report.json"
            initial = json.dumps({"results": [{"query": 1, "status": "timeout"}]})
            source.write_text(initial, encoding="utf-8")
            with self.assertRaises(SystemExit):
                main(["--input", str(source), "--output", str(source)])
            self.assertEqual(source.read_text(), initial)

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

    def test_rejects_nonfinite_milliseconds_after_unit_conversion(self):
        payload = {"results": [{
            "query": 1, "status": "passed",
            "timings_seconds": [1e308],
            "latency": {"median_seconds": 1e308, "p90_seconds": 1e308},
        }]}
        with self.assertRaisesRegex(ValueError, "millisecond values overflow"):
            rows_from_report(payload)

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

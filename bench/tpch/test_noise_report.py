import unittest
from noise_report import assess_query, assess_report


class NoiseReportTests(unittest.TestCase):
    def test_oversized_noise_report_is_rejected(self):
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from noise_report import load_report
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "noise.json"
            source.write_bytes(b"X" * 17)
            with patch("noise_report.MAX_REPORT_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    load_report(source)


    def test_loader_rejects_duplicate_keys_and_nonfinite_numbers(self):
        import tempfile
        from pathlib import Path
        from noise_report import load_report
        payloads = [
            '{"results": [], "results": []}',
            '{"results": [{"query": 1, "query": 2}]}',
            '{"results": [{"query": 1, "timings_seconds": [NaN]}]}',
            '{"results": [{"query": 1, "timings_seconds": [Infinity]}]}',
            '{"results": [{"query": 1, "timings_seconds": [-Infinity]}]}',
        ]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "noise.json"
            for payload in payloads:
                with self.subTest(payload=payload):
                    source.write_text(payload, encoding="utf-8")
                    with self.assertRaises(ValueError):
                        load_report(source)
            source.write_text('{"results": [{"query": 1, "status": "passed", '
                              '"timings_seconds": [1.0, 2.0]}]}', encoding="utf-8")
            self.assertEqual(load_report(source)["results"][0]["query"], 1)

    def test_rejects_report_swapped_before_open(self):
        import os
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from noise_report import load_report

        with tempfile.TemporaryDirectory() as root:
            original = Path(root) / "noise.json"
            replacement = Path(root) / "replacement.json"
            original.write_text('{"results":[]}', encoding="utf-8")
            replacement.write_text('{"results":[1]}', encoding="utf-8")
            real_open = os.open

            def swap(path, flags):
                original.unlink()
                replacement.rename(original)
                return real_open(path, flags)

            with patch("noise_report.os.open", side_effect=swap):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    load_report(original)

    def test_rejects_overflowed_json_float(self):
        import tempfile
        from pathlib import Path
        from noise_report import load_report

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "noise.json"
            path.write_text('{"results":[{"query":1,"seconds":1e400}]}',
                            encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "nonfinite"):
                load_report(path)

    def test_atomic_output_survives_failed_report_validation(self):
        import json
        import tempfile
        from pathlib import Path
        from noise_report import main

        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "noise.json"
            output = Path(root) / "noise-result.json"
            output.write_text("previous result", encoding="utf-8")
            path.write_text(json.dumps({"results":[]}), encoding="utf-8")
            with self.assertRaises(SystemExit):
                main(["--input", str(path), "--output", str(output)])
            self.assertEqual(output.read_text(), "previous result")

    def test_marks_repeatable_verified_samples_as_stable(self):
        result = assess_query({"query": 6, "status": "passed",
                               "timings_seconds": [1, 1, 1, 1, 1]})
        self.assertEqual(result["state"], "stable")
        self.assertEqual(result["cv_percent"], 0)
        self.assertEqual(result["mad_seconds"], 0)

    def test_marks_noisy_and_insufficient_measurements(self):
        report = assess_report({"results": [
            {"query": 6, "status": "passed", "timings_seconds": [1, 2, 3, 4, 5]},
            {"query": 1, "status": "passed", "timings_seconds": [1, 2]},
            {"query": 3, "status": "unverified", "timings_seconds": [1] * 5},
        ]})
        self.assertEqual([q["state"] for q in report["queries"]],
                         ["noisy", "insufficient_samples", "not_verified"])

    def test_large_finite_samples_do_not_overflow_cv(self):
        samples = [1e308, 1e308, 1e308, 1e308, 1e307]
        result = assess_query({"query": 6, "status": "passed",
                               "timings_seconds": samples})
        self.assertEqual(result["state"], "noisy")
        self.assertTrue(0 < result["cv_percent"] < 100)

    def test_zero_negative_and_boolean_timings_are_invalid(self):
        for bad in (0.0, -0.1, True, float("inf")):
            with self.subTest(sample=bad):
                row = assess_query({
                    "query": 6,
                    "status": "passed",
                    "timings_seconds": [1.0, 1.0, bad, 1.0, 1.0],
                })
                self.assertEqual(row["state"], "invalid_samples")

    def test_rejects_empty_reports_that_would_bypass_the_gate(self):
        with self.assertRaisesRegex(ValueError, "invalid TPC-H report"):
            assess_report({"results": []})

    def test_rejects_fractional_sample_counts_and_boolean_thresholds(self):
        for samples in (True, 2.5):
            with self.assertRaisesRegex(ValueError, "thresholds"):
                assess_query({"query": 1}, min_samples=samples)
        for threshold in (True, float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "thresholds"):
                assess_query({"query": 1}, max_cv_percent=threshold)

    def test_rejects_invalid_thresholds_and_samples(self):
        with self.assertRaises(ValueError):
            assess_query({"query": 1}, min_samples=1)
        invalid = assess_query({"query": 1, "status": "passed",
                                "timings_seconds": [float("nan")] * 5})
        self.assertEqual(invalid["state"], "invalid_samples")


class NoiseReportQueryIdTests(unittest.TestCase):
    def test_rejects_duplicate_and_invalid_query_identifiers(self):
        for results in (
            [{"query": 6, "status": "passed"}, {"query": 6, "status": "passed"}],
            [{"query": True, "status": "passed"}],
            [{"query": "6", "status": "passed"}],
            [None],
        ):
            with self.subTest(results=results):
                with self.assertRaisesRegex(ValueError, "distinct positive"):
                    assess_report({"results": results})

if __name__ == "__main__":
    unittest.main()

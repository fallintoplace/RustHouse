import importlib.util
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "release_gate.py"
SPEC = importlib.util.spec_from_file_location("rusthouse_release_gate", SCRIPT)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


def report(results, runs=2, reference_checked=True, timing_key="timings_seconds"):
    return {
        "reference_checked": reference_checked,
        "runs": runs,
        "results": [
            {"query": number, "status": status, timing_key: [0.01, 0.02]}
            for number, status in results
        ],
    }


class ReleaseGateTests(unittest.TestCase):
    def test_rejects_duplicate_verification_fields_in_manifest(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_text(
                '{"reference_checked": false, "reference_checked": true, '
                '"runs":1,"results":[]}', encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "duplicate benchmark manifest key"):
                gate.load_manifest(path)

    def test_large_release_report_is_rejected(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "report.json"
            path.write_bytes(b"X" * 17)
            with patch.object(gate, "MAX_MANIFEST_BYTES", 16):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    gate.load_manifest(path)


    def test_accepts_verified_tpch_and_analytics_reports(self):
        required = {1, 6}
        gate.verify(report([(1, "passed"), (6, "passed")]), required)
        gate.verify(
            report([(6, "passed"), (1, "passed")], timing_key="seconds"),
            required,
        )

    def test_required_commit_revision_rejects_stale_benchmark_data(self):
        sample = report([(1, "passed")])
        sample["revision"] = "0123456789abcdef"
        gate.verify(sample, {1}, expected_revision="0123456789abcdef")
        for revision in ("old-sha", "", None):
            with self.subTest(revision=revision):
                stale = dict(sample)
                stale["revision"] = revision
                with self.assertRaisesRegex(ValueError, "revision differs"):
                    gate.verify(stale, {1}, expected_revision="0123456789abcdef")

    def test_parses_exact_required_ids_and_ranges(self):
        self.assertEqual(gate.parse_query_ids("1-3,6,10-11"), {1, 2, 3, 6, 10, 11})
        for specification in ["", "0", "4-2", "1,1", "2-4,3", "abc", "1--2"]:
            with self.subTest(specification=specification):
                with self.assertRaises(ValueError):
                    gate.parse_query_ids(specification)

    def test_fails_on_missing_extra_or_duplicate_queries(self):
        for rows in [
            [(1, "passed")],
            [(1, "passed"), (2, "passed")],
            [(1, "passed"), (1, "passed")],
        ]:
            with self.subTest(rows=rows):
                with self.assertRaises(ValueError):
                    gate.verify(report(rows), {1, 6})

    def test_fails_on_unverified_or_incorrect_output(self):
        for status in ["unverified", "mismatch", "missing_reference", "error", "timeout"]:
            with self.subTest(status=status):
                with self.assertRaisesRegex(ValueError, "not reference-verified"):
                    gate.verify(report([(1, status)]), {1})
        with self.assertRaisesRegex(ValueError, "reference comparisons"):
            gate.verify(report([(1, "passed")], reference_checked=False), {1})

    def test_minimum_timing_runs_enforces_sample_confidence(self):
        candidate = report([(1, "passed")], runs=2)
        gate.verify(candidate, {1}, min_runs=2)
        with self.assertRaisesRegex(ValueError, "fewer than required"):
            gate.verify(candidate, {1}, min_runs=3)
        for invalid in (0, True, -1, 1001):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, "min_runs"):
                gate.verify(candidate, {1}, min_runs=invalid)

    def test_rejects_incomplete_or_invalid_samples(self):
        baseline = report([(1, "passed")])
        baseline["results"][0]["timings_seconds"] = [0.01]
        with self.assertRaisesRegex(ValueError, "incomplete timing"):
            gate.verify(baseline, {1})
        for invalid in [float("nan"), float("inf"), -1, True, "slow"]:
            with self.subTest(invalid=invalid):
                bad = report([(1, "passed")])
                bad["results"][0]["timings_seconds"][0] = invalid
                with self.assertRaisesRegex(ValueError, "invalid timing"):
                    gate.verify(bad, {1})

    def test_verified_latency_baseline_blocks_regressions(self):
        baseline = report([(1, "passed"), (6, "passed")])
        candidate = report([(1, "passed"), (6, "passed")])
        candidate["results"][0]["timings_seconds"] = [0.10, 0.20]
        with self.assertRaisesRegex(ValueError, "median latency"):
            gate.verify_latency_baseline(candidate, baseline, {1, 6}, 1.25)
        candidate["results"][0]["timings_seconds"] = [0.01, 0.02]
        gate.verify_latency_baseline(candidate, baseline, {1, 6}, 1.25)
        with self.assertRaisesRegex(ValueError, "max slowdown"):
            gate.verify_latency_baseline(candidate, baseline, {1, 6}, float("nan"))


    def test_cli_rejects_unverified_report_and_accepts_verified_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "report.json"
            path.write_text(json.dumps(report([(6, "passed")])), encoding="utf-8")
            with redirect_stdout(StringIO()):
                self.assertEqual(gate.main([str(path), "--queries", "6"]), 0)
            path.write_text(json.dumps(report([(6, "unverified")])), encoding="utf-8")
            with redirect_stderr(StringIO()):
                self.assertEqual(gate.main([str(path), "--queries", "6"]), 1)


if __name__ == "__main__":
    unittest.main()

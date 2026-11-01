import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from run_canonical import MAX_QUERY_BYTES, command, read_bounded, run_full


def corpus(root):
    root.mkdir()
    for i in range(1, 23):
        (root / f"q{i:02d}.sql").write_text(
            f"-- Full TPC-H Q{i} source\n-- SPDX-License-Identifier: MIT\nSELECT {i};\n"
        )


class FullRunnerTests(unittest.TestCase):
    def test_bounded_query_and_report_inputs(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "q01.sql"
            path.write_bytes(b"X" * 17)
            with self.assertRaisesRegex(ValueError, "exceeds"):
                read_bounded(path, 16)
            self.assertEqual(read_bounded(path, 17), "X" * 17)


    def test_requires_22_upstream_labeled_queries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "queries"
            corpus(root)
            cmd = command("rusthouse", "db", root, "reference", "output.json")
            self.assertIn("--require-22", cmd)
            self.assertIn("--require-verified", cmd)
            self.assertIn("6001215", cmd)
            (root / "q22.sql").unlink()
            with self.assertRaisesRegex(ValueError, "full canonical"):
                command("rusthouse", "db", root, "reference", "output.json")

    def test_rejects_float_bool_and_duplicate_report_query_ids(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "queries"
            corpus(root)
            report_path = Path(tmp) / "report.json"
            for invalid in (True, 1.0, "1"):
                def invoke(cmd):
                    report_path.write_text(json.dumps({
                        "reference_checked": True,
                        "results": [
                            {"query": invalid if i == 1 else i, "status": "passed"}
                            for i in range(1, 23)
                        ],
                    }), encoding="utf-8")
                    return subprocess.CompletedProcess(cmd, 0)

                with self.subTest(id=invalid):
                    with self.assertRaisesRegex(ValueError, "invalid or duplicate"):
                        run_full("rusthouse", "db", root, "refs", report_path,
                                 invoke=invoke)

    def test_rejects_canonical_query_replaced_during_open(self):
        import os
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "q01.sql"
            alternate = Path(root) / "other.sql"
            path.write_text("SELECT 1", encoding="utf-8")
            alternate.write_text("SELECT 2", encoding="utf-8")
            real_open = os.open

            def change(filename, flags):
                path.unlink()
                alternate.rename(path)
                return real_open(filename, flags)

            with patch("run_canonical.os.open", side_effect=change):
                with self.assertRaisesRegex(ValueError, "changed while opening"):
                    read_bounded(path, MAX_QUERY_BYTES)

    def test_reports_incomplete_queries_without_claiming_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "queries"
            corpus(root)
            report_path = Path(tmp) / "report.json"
            def invoke(cmd):
                report_path.write_text(json.dumps({
                    "reference_checked": True,
                    "results": [{"query": i, "status": "passed" if i != 17 else "error"}
                                for i in range(1, 23)],
                }))
                return subprocess.CompletedProcess(cmd, 1)
            result = run_full("rusthouse", "db", root, "refs", report_path,
                              invoke=invoke)
            self.assertFalse(result["complete"])
            self.assertEqual(result["failed"], [{"query": 17, "status": "error"}])


class CanonicalTimingControlsTests(unittest.TestCase):
    def test_rejects_invalid_controls_before_loading_sql(self):
        for options in ({"runs": True}, {"runs": 1.5},
                        {"warmups": False}, {"timeout": float("nan")},
                        {"timeout": float("inf")}, {"timeout": True}):
            with self.subTest(options=options):
                with self.assertRaisesRegex(ValueError, "invalid timing options"):
                    command("rusthouse", "db", Path("absent-queries"),
                            "refs", "report.json", **options)

if __name__ == "__main__":
    unittest.main()

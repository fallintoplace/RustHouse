import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from run_canonical import command, run_full


def corpus(root):
    root.mkdir()
    for i in range(1, 23):
        (root / f"q{i:02d}.sql").write_text(
            f"-- Full TPC-H Q{i} source\n-- SPDX-License-Identifier: MIT\nSELECT {i};\n"
        )


class FullRunnerTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()

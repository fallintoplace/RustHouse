import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from run_canonical import run_full


class CanonicalFailureSummaryTests(unittest.TestCase):
    def test_keeps_first_sql_failure_in_failed_query_summary(self):
        with tempfile.TemporaryDirectory() as root:
            queries = Path(root) / "queries"
            queries.mkdir()
            for number in range(1, 23):
                (queries / f"q{number:02d}.sql").write_text(
                    f"-- Full TPC-H Q{number} source\n-- SPDX-License-Identifier: MIT\nSELECT {number};\n")
            destination = Path(root) / "results.json"

            def invoke(cmd):
                results = [{"query": n, "status": "passed"} for n in range(1, 23)]
                results[0] = {"query": 1, "status": "error",
                              "error": "expected ')' after grouped aggregate"}
                results[3] = {"query": 4, "status": "error", "error": "x" * 600}
                destination.write_text(json.dumps({
                    "reference_checked": True, "results": results,
                }))
                return subprocess.CompletedProcess(cmd, 1)

            report = run_full("unused", "unused", queries, root, destination, invoke=invoke)
            self.assertFalse(report["complete"])
            self.assertEqual(report["failed"][0], {
                "query": 1, "status": "error",
                "error": "expected ')' after grouped aggregate",
            })
            self.assertEqual(len(report["failed"][1]["error"]), 500)
            self.assertEqual(report["verified_queries"], [n for n in range(2, 23) if n != 4])


if __name__ == "__main__":
    unittest.main()

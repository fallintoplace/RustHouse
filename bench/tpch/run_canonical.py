#!/usr/bin/env python3
"""Execute the full pinned 22-query TPC-H corpus with required reference checking."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

from run import load_queries, official_22_coverage


def command(binary, database, queries_dir, reference_dir, report_path,
            runs=3, warmups=1, timeout=300):
    if runs < 1 or warmups < 0 or timeout <= 0:
        raise ValueError("invalid timing options")
    queries_dir = Path(queries_dir)
    queries = load_queries(queries_dir)
    inventory = official_22_coverage(queries)
    if inventory["available_queries"] != 22 or inventory["missing_query_ids"] or inventory["unexpected_query_ids"]:
        raise ValueError("full canonical q01.sql through q22.sql corpus required")
    for number in range(1, 23):
        content = (queries_dir / f"q{number:02d}.sql").read_text(encoding="utf-8")
        first_lines = content.splitlines()[:2]
        if len(first_lines) != 2 or "TPC-H Q" not in first_lines[0] or "SPDX-License-Identifier: MIT" not in first_lines[1]:
            raise ValueError(f"q{number:02d}: missing canonical upstream provenance")
    return [sys.executable, str(Path(__file__).with_name("run.py")),
            "--binary", str(binary),
            "--database", str(database),
            "--queries-dir", str(queries_dir),
            "--reference-dir", str(reference_dir),
            "--require-22", "--require-verified",
            "--comparison", "unordered",
            "--expected-lineitem-rows", "6001215",
            "--runs", str(runs), "--warmups", str(warmups),
            "--timeout", str(timeout), "--output", str(report_path)]


def run_full(binary, database, queries_dir, reference_dir, report_path,
             runs=3, warmups=1, timeout=300, invoke=None):
    if invoke is None:
        invoke = lambda cmd: subprocess.run(cmd, check=False)
    cmd = command(binary, database, queries_dir, reference_dir,
                  report_path, runs, warmups, timeout)
    outcome = invoke(cmd)
    if not Path(report_path).is_file():
        raise ValueError("full-query runner did not produce a JSON report")
    report = json.loads(Path(report_path).read_text(encoding="utf-8"))
    records = report.get("results", [])
    verified = sorted(record["query"] for record in records
                      if record.get("status") == "passed")
    complete = (outcome.returncode == 0 and report.get("reference_checked") is True
                and verified == list(range(1, 23)) and len(records) == 22)
    return {"complete": complete, "verified_queries": verified,
            "failed": [{"query": record["query"], "status": record["status"]}
                       for record in records if record.get("status") != "passed"],
            "runner_exit_code": outcome.returncode,
            "report": str(report_path), "official_tpch_score": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path,
                        default=Path(__file__).resolve().parent / "queries")
    parser.add_argument("--reference-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=300)
    args = parser.parse_args(argv)
    try:
        report = run_full(args.binary, args.database, args.queries_dir,
                          args.reference_dir, args.output, args.runs,
                          args.warmups, args.timeout)
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        parser.exit(1, f"full TPC-H run failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

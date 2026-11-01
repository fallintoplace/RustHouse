#!/usr/bin/env python3
"""Execute the full pinned 22-query TPC-H corpus with required reference checking."""
import argparse
import json
import math
import os
import stat
import subprocess
import sys
from pathlib import Path

from run import load_queries, official_22_coverage

MAX_QUERY_BYTES = 64 * 1024
MAX_REPORT_BYTES = 2 * 1024 * 1024


def read_bounded(path, limit):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("canonical input must be a regular, non-symlinked file")

    def identity(metadata):
        return (metadata.st_dev, metadata.st_ino, metadata.st_size,
                metadata.st_mtime_ns, metadata.st_ctime_ns)

    fd = os.open(path, os.O_RDONLY | getattr(os, "O_BINARY", 0)
                 | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or identity(opened) != identity(before):
            raise ValueError("canonical input changed while opening")
        content = stream.read(limit + 1)
        if identity(opened) != identity(os.fstat(stream.fileno())):
            raise ValueError("canonical input changed while reading")
    if identity(before) != identity(path.lstat()):
        raise ValueError("canonical input changed while reading")
    if len(content) > limit:
        raise ValueError("canonical input exceeds its byte limit")
    return content.decode("utf-8")



def command(binary, database, queries_dir, reference_dir, report_path,
            runs=3, warmups=1, timeout=300):
    if (type(runs) is not int or runs < 1
            or type(warmups) is not int or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("invalid timing options")
    queries_dir = Path(queries_dir)
    queries = load_queries(queries_dir)
    inventory = official_22_coverage(queries)
    if inventory["available_queries"] != 22 or inventory["missing_query_ids"] or inventory["unexpected_query_ids"]:
        raise ValueError("full canonical q01.sql through q22.sql corpus required")
    for number in range(1, 23):
        content = read_bounded(queries_dir / f"q{number:02d}.sql", MAX_QUERY_BYTES)
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


def parse_report(path):
    def unique_fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate full TPC-H report JSON field: {key}")
            result[key] = value
        return result

    def reject_nonstandard_number(token):
        raise ValueError(f"nonstandard full TPC-H report JSON number: {token}")

    return json.loads(read_bounded(path, MAX_REPORT_BYTES),
                      object_pairs_hook=unique_fields,
                      parse_constant=reject_nonstandard_number)


def run_full(binary, database, queries_dir, reference_dir, report_path,
             runs=3, warmups=1, timeout=300, invoke=None):
    if invoke is None:
        invoke = lambda cmd: subprocess.run(cmd, check=False)
    cmd = command(binary, database, queries_dir, reference_dir,
                  report_path, runs, warmups, timeout)
    outcome = invoke(cmd)
    if not Path(report_path).is_file():
        raise ValueError("full-query runner did not produce a JSON report")
    report = parse_report(report_path)
    records = report.get("results")
    if not isinstance(records, list):
        raise ValueError("full-query report requires a results array")
    verified = []
    failed = []
    seen = set()
    for record in records:
        if not isinstance(record, dict):
            raise ValueError("full-query report contains an invalid result")
        number, status = record.get("query"), record.get("status")
        # A boolean or 1.0 compares equal to integer 1 in Python. Such IDs
        # must not count toward full 22-query reference verification.
        if (type(number) is not int or not 1 <= number <= 22
                or number in seen or not isinstance(status, str)):
            raise ValueError("full-query report has invalid or duplicate query results")
        seen.add(number)
        if status == "passed":
            verified.append(number)
        else:
            failed.append({"query": number, "status": status})
    verified.sort()
    complete = (outcome.returncode == 0 and report.get("reference_checked") is True
                and verified == list(range(1, 23)) and len(records) == 22)
    return {"complete": complete, "verified_queries": verified,
            "failed": failed,
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

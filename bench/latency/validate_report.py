"""Validate the full generated-SQL latency report before CI publishes it."""

import argparse
import json
import math
import os
import stat
import sys
from pathlib import Path

MAX_REPORT_BYTES = 2 * 1024 * 1024


def unique_object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate SQL latency report key: {key}")
        result[key] = value
    return result


def load_report(path):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("SQL latency report must be a regular non-symlinked file")

    def identity(info):
        return (info.st_dev, info.st_ino, info.st_size,
                info.st_mtime_ns, info.st_ctime_ns)

    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or identity(opened) != identity(before):
            raise ValueError("SQL latency report changed before opening")
        payload = source.read(MAX_REPORT_BYTES + 1)
        if identity(opened) != identity(os.fstat(source.fileno())):
            raise ValueError("SQL latency report changed while reading")
    if identity(before) != identity(path.lstat()):
        raise ValueError("SQL latency report changed while reading")
    if len(payload) > MAX_REPORT_BYTES:
        raise ValueError("SQL latency report exceeds 2 MiB")

    def reject_nonfinite(value):
        raise ValueError(f"nonstandard latency JSON constant: {value}")

    return json.loads(payload.decode("utf-8"), object_pairs_hook=unique_object_pairs,
                      parse_constant=reject_nonfinite)


from bench.common import latency_summary
from bench.latency.run import make_cases


def validate_query_record(index, case, result, runs):
    if not isinstance(result, dict):
        raise ValueError(f"query {index} has an invalid record")
    if (type(result.get("id")) is not int or result["id"] != index
            or result.get("name") != case.name or result.get("sql") != case.sql):
        raise ValueError(f"query {index} does not match the expected generated SQL")
    if result.get("status") != "passed" or result.get("error") is not None:
        raise ValueError(f"query {index} did not pass exact-output verification")
    samples = result.get("seconds")
    if not isinstance(samples, list) or len(samples) != runs:
        raise ValueError(f"query {index} has incomplete timing samples")
    if any(type(value) not in (int, float) or not math.isfinite(value) or value < 0
           for value in samples):
        raise ValueError(f"query {index} has invalid timing samples")
    if result.get("latency") != latency_summary(samples):
        raise ValueError(f"query {index} has inconsistent latency statistics")


def validate_report(report, *, rows, runs, warmups):
    if (type(rows) is not int or rows < 1
            or type(runs) is not int or runs < 1
            or type(warmups) is not int or warmups < 0):
        raise ValueError("rows and runs must be positive integers; warmups nonnegative")
    if not isinstance(report, dict) or report.get("suite") != "generated-sql-latency":
        raise ValueError("wrong SQL latency report type")
    for name, expected in (("rows", rows), ("runs", runs), ("warmups", warmups)):
        if type(report.get(name)) is not int or report[name] != expected:
            raise ValueError(f"expected {name}={expected}")

    expected_cases = make_cases(rows)
    if len(expected_cases) != 20:
        raise ValueError(f"expected 20 generated queries, got {len(expected_cases)}")
    results = report.get("results")
    if not isinstance(results, list) or len(results) != len(expected_cases):
        raise ValueError("latency report must contain exactly 20 query results")

    for index, (case, result) in enumerate(zip(expected_cases, results), start=1):
        validate_query_record(index, case, result, runs)
    return len(results)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--runs", type=int, required=True)
    parser.add_argument("--warmups", type=int, required=True)
    args = parser.parse_args(argv)
    try:
        report = load_report(args.report)
        count = validate_report(report, rows=args.rows, runs=args.runs, warmups=args.warmups)
    except (OSError, ValueError) as error:
        print(f"SQL latency report validation failed: {error}", file=sys.stderr)
        return 1
    print(f"SQL latency report validated: {count} verified queries with complete timings")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

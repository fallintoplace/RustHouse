"""Validate the full generated-SQL latency report before CI publishes it."""

import argparse
import json
import math
import sys
from pathlib import Path

MAX_REPORT_BYTES = 2 * 1024 * 1024


def load_report(path):
    with Path(path).open("rb") as source:
        payload = source.read(MAX_REPORT_BYTES + 1)
    if len(payload) > MAX_REPORT_BYTES:
        raise ValueError("SQL latency report exceeds 2 MiB")
    return json.loads(payload.decode("utf-8"))


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

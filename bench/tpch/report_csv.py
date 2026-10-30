#!/usr/bin/env python3
"""Export RustHouse TPC-H SQL benchmark JSON results to CSV."""
import argparse
import csv
import json
import math
import statistics
import sys
from pathlib import Path

FIELDS = ("query", "status", "samples", "median_ms", "p90_ms", "error")
KNOWN_STATUSES = frozenset((
    "passed", "unverified", "missing_query", "missing_reference",
    "unsupported_query", "mismatch", "timeout", "reference_error", "error",
))


def safe_csv_error(value):
    text = "" if value is None else str(value)
    if text.lstrip().startswith(("=", "+", "-", "@")) or text.startswith(("\t", "\r", "\n")):
        return "'" + text
    return text


def rows_from_report(report):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("report must contain a results array")
    if not report["results"]:
        raise ValueError("report must contain query results")
    rows = []
    seen = set()
    for result in report["results"]:
        if not isinstance(result, dict) or "query" not in result or "status" not in result:
            raise ValueError("each result needs query and status")
        if (not isinstance(result["status"], str)
                or result["status"] not in KNOWN_STATUSES):
            raise ValueError("unknown benchmark query status")
        number = result["query"]
        if type(number) is not int or number < 1 or number in seen:
            raise ValueError("query IDs must be distinct positive integers")
        seen.add(number)
        values = result.get("timings_seconds", [])
        latency = result.get("latency")
        if not isinstance(values, list):
            raise ValueError("timings_seconds must be a list")
        valid = result["status"] in ("passed", "unverified")
        if valid:
            if not values or any(
                type(value) not in (int, float) or not math.isfinite(value) or value <= 0
                for value in values
            ):
                raise ValueError("successful query has invalid timing samples")
            if not isinstance(latency, dict) or any(
                type(latency.get(key)) not in (int, float)
                or not math.isfinite(latency[key])
                or latency[key] <= 0
                for key in ("median_seconds", "p90_seconds")
            ):
                raise ValueError("successful query has invalid latency summary")
        if valid:
            median = statistics.median(values)
            p90 = sorted(values)[math.ceil(0.9 * len(values)) - 1]
            if (not math.isclose(latency["median_seconds"], median, rel_tol=1e-9)
                    or not math.isclose(latency["p90_seconds"], p90, rel_tol=1e-9)):
                raise ValueError("successful query has inconsistent latency summary")
        rows.append({
            "query": result["query"],
            "status": result["status"],
            "samples": len(values) if valid else 0,
            "median_ms": (f'{latency["median_seconds"] * 1000:.6f}'
                          if valid and latency else ""),
            "p90_ms": (f'{latency["p90_seconds"] * 1000:.6f}'
                       if valid and latency else ""),
            "error": safe_csv_error(result.get("error")),
        })
    return rows


def write_csv(report, output):
    writer = csv.DictWriter(output, fieldnames=FIELDS, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows_from_report(report))



MAX_REPORT_BYTES = 16 * 1024 * 1024


def _read_bounded_report(path):
    with path.open("rb") as source:
        contents = source.read(MAX_REPORT_BYTES + 1)
    if len(contents) > MAX_REPORT_BYTES:
        raise ValueError("benchmark JSON report exceeds 16 MiB")
    return contents.decode("utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = json.loads(_read_bounded_report(args.input))
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open("w", encoding="utf-8", newline="") as output:
                write_csv(report, output)
        else:
            write_csv(report, sys.stdout)
    except (OSError, ValueError, TypeError, KeyError) as error:
        parser.exit(1, f"CSV export failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

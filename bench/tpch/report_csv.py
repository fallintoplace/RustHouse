#!/usr/bin/env python3
"""Export RustHouse TPC-H SQL benchmark JSON results to CSV."""
import argparse
import csv
import json
import sys
from pathlib import Path

FIELDS = ("query", "status", "samples", "median_ms", "p90_ms", "error")


def rows_from_report(report):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("report must contain a results array")
    rows = []
    for result in report["results"]:
        if not isinstance(result, dict) or "query" not in result or "status" not in result:
            raise ValueError("each result needs query and status")
        values = result.get("timings_seconds") or []
        latency = result.get("latency")
        if not isinstance(values, list):
            raise ValueError("timings_seconds must be a list")
        valid = result["status"] in ("passed", "unverified")
        rows.append({
            "query": result["query"],
            "status": result["status"],
            "samples": len(values) if valid else 0,
            "median_ms": (f'{latency["median_seconds"] * 1000:.6f}'
                          if valid and latency else ""),
            "p90_ms": (f'{latency["p90_seconds"] * 1000:.6f}'
                       if valid and latency else ""),
            "error": result.get("error") or "",
        })
    return rows


def write_csv(report, output):
    writer = csv.DictWriter(output, fieldnames=FIELDS, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows_from_report(report))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = json.loads(args.input.read_text(encoding="utf-8"))
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

#!/usr/bin/env python3
"""Compare two 22-query TPC-H compatibility reports without claiming a TPC score."""
import argparse
import json
from pathlib import Path

from feature_gaps import load_report

STATUSES = frozenset((
    "passed", "unverified", "missing_query", "missing_reference",
    "unsupported_query", "mismatch", "timeout", "reference_error", "error",
))
LEVEL = {"passed": 2, "unverified": 1}


def _statuses(report):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("TPC-H report requires results array")
    result = {}
    for row in report["results"]:
        if not isinstance(row, dict):
            raise ValueError("TPC-H results must contain objects")
        number, status = row.get("query"), row.get("status")
        if type(number) is not int or not 1 <= number <= 22 or number in result:
            raise ValueError("TPC-H query IDs must be unique integers 1..22")
        if status not in STATUSES:
            raise ValueError("TPC-H report contains unknown status")
        result[number] = status
    if len(result) != 22:
        raise ValueError("TPC-H report must contain all 22 queries")
    return result


def compare(before, after):
    older = _statuses(before)
    newer = _statuses(after)
    changes = []
    counts = {"improved": 0, "regressed": 0, "changed": 0, "unchanged": 0}
    for number in range(1, 23):
        prev, curr = older[number], newer[number]
        delta = LEVEL.get(curr, 0) - LEVEL.get(prev, 0)
        kind = ("improved" if delta > 0 else "regressed" if delta < 0
                else "unchanged" if curr == prev else "changed")
        counts[kind] += 1
        if kind != "unchanged":
            changes.append({"query": number, "before": prev,
                            "after": curr, "change": kind})
    return {"workload": "tpch-coverage-delta", "official_tpch_score": False,
            "counts": counts, "verified_before": sum(v == "passed" for v in older.values()),
            "verified_after": sum(v == "passed" for v in newer.values()),
            "changes": changes}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = compare(load_report(args.before), load_report(args.after))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, f"TPC-H coverage comparison failed: {error}\n")
    payload = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return 1 if result["counts"]["regressed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())

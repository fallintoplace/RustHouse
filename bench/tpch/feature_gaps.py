#!/usr/bin/env python3
"""Summarize TPC-H query execution failures by likely blocker category.

Categories are diagnostic hints based on error messages, not a SQL support spec.
"""
import argparse
import json
import re
from collections import defaultdict
from pathlib import Path

RULES = (
    ("join_capacity", re.compile(r"JOIN (?:supports at most|exceeds its|input row limit)", re.I)),
    ("join_syntax", re.compile(r"JOIN columns must|unknown JOIN qualifier|JOIN key types differ", re.I)),
    ("sql_size", re.compile(r"(?:SQL|statement).*(?:4096|4.?KiB|size limit|too long|exceeds)", re.I)),
    ("grouping", re.compile(r"GROUP BY (?:supports|requires|cannot|must)", re.I)),
    ("resource_limit", re.compile(
        r"(?:exceeds|too many|more than|limited to|at most).{0,80}"
        r"(?:rows|bytes|MiB|GiB|memory|spill|budget|parts)", re.I)),
    ("unimplemented_operator", re.compile(r"(?:unsupported|not implemented|not supported)", re.I)),
    ("missing_column", re.compile(r"(?:unknown|missing|undefined) column", re.I)),
)


KNOWN_STATUSES = frozenset((
    "passed", "unverified", "missing_query", "missing_reference",
    "unsupported_query", "mismatch", "timeout", "reference_error",
))


def classify_error(message):
    for label, pattern in RULES:
        if pattern.search(message):
            return label
    return "unclassified_error"


def classify(result):
    status = result.get("status", "unknown")
    if not isinstance(status, str):
        return "unexpected_status"
    if status in KNOWN_STATUSES:
        return status
    if status != "error":
        return "unexpected_status"
    return classify_error(str(result.get("error") or ""))


def analyze(report, require_all_22=False):
    if not isinstance(report, dict):
        raise ValueError("report must be an object")
    records = report.get("results")
    if not isinstance(records, list):
        raise ValueError("report must contain a results array")
    seen = set()
    groups = defaultdict(list)
    for item in records:
        if not isinstance(item, dict):
            raise ValueError("each result must be an object")
        number = item.get("query")
        if type(number) is not int or not 1 <= number <= 22 or number in seen:
            raise ValueError("query IDs must be unique integers in 1..22")
        if "status" in item and not isinstance(item["status"], str):
            raise ValueError("query result status must be a string")
        seen.add(number)
        groups[classify(item)].append(number)
    if require_all_22 and seen != set(range(1, 23)):
        raise ValueError("report must include each query ID from 1 to 22")
    executed = sum(len(groups[label]) for label in ("passed", "unverified"))
    verified = len(groups["passed"])
    total = len(records)
    return {
        "workload": "tpch-compatibility-gaps",
        "official_tpch_score": False,
        "queries_examined": total,
        "execution_success_percent": round(100 * executed / total, 2) if total else 0,
        "verified_correct_percent": round(100 * verified / total, 2) if total else 0,
        "categories": [
            {"category": label, "count": len(numbers), "queries": sorted(numbers)}
            for label, numbers in sorted(groups.items(), key=lambda pair: (-len(pair[1]), pair[0]))
            if numbers
        ],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--require-all-22", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        source = json.loads(args.report.read_text(encoding="utf-8"))
        result = analyze(source, args.require_all_22)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.exit(1, f"compatibility report failed: {error}\n")
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

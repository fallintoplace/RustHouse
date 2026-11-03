#!/usr/bin/env python3
"""Join canonical TPC-H parser and execution reports without conflating success."""
import argparse
import json
from pathlib import Path

from feature_gaps import load_report

QUERY_IDS = frozenset(range(1, 23))
PARSER_STATUSES = frozenset(("parsed", "parse_error", "invalid_query", "missing_query"))
EXECUTION_STATUSES = frozenset((
    "passed", "unverified", "missing_query", "missing_reference",
    "unsupported_query", "mismatch", "timeout", "reference_error", "error",
))


def _index(report, statuses, label):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError(f"{label} report requires results array")
    result = {}
    for row in report["results"]:
        if not isinstance(row, dict):
            raise ValueError(f"{label} query entry must be an object")
        number, status = row.get("query"), row.get("status")
        if type(number) is not int or number not in QUERY_IDS or number in result:
            raise ValueError(f"{label} query IDs must be distinct integers 1..22")
        if status not in statuses:
            raise ValueError(f"{label} has unknown query status")
        result[number] = row
    if result.keys() != QUERY_IDS:
        raise ValueError(f"{label} must contain all 22 canonical query IDs")
    return result


def combine(parser_report, execution_report):
    """Report exact per-stage status for every canonical TPC-H query."""
    parsed = _index(parser_report, PARSER_STATUSES, "parser")
    executed = _index(execution_report, EXECUTION_STATUSES, "execution")
    rows = []
    categories = {"not_parsed": 0, "execution_failed": 0, "unverified": 0,
                  "verified": 0, "inconsistent": 0}
    for number in range(1, 23):
        parse_status = parsed[number]["status"]
        run_status = executed[number]["status"]
        if parse_status != "parsed":
            category = ("inconsistent" if run_status in ("passed", "unverified")
                        else "not_parsed")
        elif run_status == "passed":
            category = "verified"
        elif run_status == "unverified":
            category = "unverified"
        else:
            category = "execution_failed"
        categories[category] += 1
        rows.append({"query": number, "parser": parse_status,
                     "execution": run_status, "stage": category})
    return {"workload": "tpch-stage-coverage", "official_tpch_score": False,
            "total_queries": 22, "categories": categories, "results": rows}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parser-report", type=Path, required=True)
    parser.add_argument("--execution-report", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = combine(load_report(args.parser_report),
                         load_report(args.execution_report))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, f"TPC-H stage coverage failed: {error}\n")
    payload = json.dumps(result, indent=2) + "\n"
    if args.output is not None:
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return 0 if result["categories"]["verified"] == 22 else 1


if __name__ == "__main__":
    raise SystemExit(main())

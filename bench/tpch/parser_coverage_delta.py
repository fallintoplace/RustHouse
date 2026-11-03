#!/usr/bin/env python3
"""Compare canonical TPC-H parser inventories without claiming executed correctness."""
import argparse
import json
from pathlib import Path

from feature_gaps import load_report

KNOWN = frozenset(("parsed", "parse_error", "invalid_query", "missing_query"))
EXPECTED = frozenset(range(1, 23))


def _index(report):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("TPC-H parser report requires results array")
    indexed = {}
    for row in report["results"]:
        if not isinstance(row, dict):
            raise ValueError("TPC-H parser rows must be objects")
        query, status = row.get("query"), row.get("status")
        if type(query) is not int or query not in EXPECTED or query in indexed:
            raise ValueError("TPC-H parser query IDs must be distinct integers in 1..22")
        if not isinstance(status, str) or status not in KNOWN:
            raise ValueError("unknown TPC-H parser status")
        indexed[query] = row
    if indexed.keys() != EXPECTED:
        raise ValueError("TPC-H parser reports must contain all 22 queries")
    return indexed


def compare(before, after):
    old = _index(before)
    new = _index(after)
    unblocked, regressed, other_changes, error_transitions = [], [], [], []
    for query in range(1, 23):
        previous, current = old[query]["status"], new[query]["status"]
        if previous != "parsed" and current == "parsed":
            unblocked.append(query)
        elif previous == "parsed" and current != "parsed":
            regressed.append(query)
        elif previous != current:
            other_changes.append({"query": query, "before": previous, "after": current})
        elif previous == current == "parse_error":
            previous_error, current_error = old[query].get("error"), new[query].get("error")
            if (isinstance(previous_error, str) and isinstance(current_error, str)
                    and previous_error != current_error):
                error_transitions.append({
                    "query": query,
                    "before_error": previous_error[:500],
                    "after_error": current_error[:500],
                })
    return {
        "workload": "tpch-parser-coverage-delta",
        "official_tpch_score": False,
        "parsed_before": sum(row["status"] == "parsed" for row in old.values()),
        "parsed_after": sum(row["status"] == "parsed" for row in new.values()),
        "unblocked_queries": unblocked,
        "regressed_queries": regressed,
        "other_status_changes": other_changes,
        "first_error_transitions": error_transitions,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = compare(load_report(args.before), load_report(args.after))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, f"TPC-H parser delta failed: {error}\n")
    payload = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return int(bool(result["regressed_queries"]))


if __name__ == "__main__":
    raise SystemExit(main())

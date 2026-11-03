#!/usr/bin/env python3
"""Cross-reference TPC-H SQL features and verified execution, without inferring support."""
import argparse
import json
from collections import defaultdict
from pathlib import Path

from feature_gaps import KNOWN_STATUSES, load_report

EXPECTED_QUERY_IDS = frozenset(range(1, 23))
ALLOWED_STATUSES = KNOWN_STATUSES | {"error"}


def _index(report, *, is_feature):
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("TPC-H report requires a results array")
    rows = {}
    for row in report["results"]:
        if not isinstance(row, dict):
            raise ValueError("TPC-H query records must be objects")
        query = row.get("query")
        if type(query) is not int or query not in EXPECTED_QUERY_IDS or query in rows:
            raise ValueError("TPC-H query IDs must be distinct integers in 1..22")
        if is_feature:
            features = row.get("features")
            if (not isinstance(features, list) or len(features) > 64
                    or any(not isinstance(feature, str) or not feature or len(feature) > 128
                           for feature in features)
                    or len(features) != len(set(features))):
                raise ValueError("TPC-H features must be unique bounded strings")
        elif not isinstance(row.get("status"), str) or row["status"] not in ALLOWED_STATUSES:
            raise ValueError("unknown TPC-H execution status")
        rows[query] = row
    if rows.keys() != EXPECTED_QUERY_IDS:
        raise ValueError("TPC-H reports must contain all 22 queries")
    return rows


def summarize(feature_report, execution_report):
    """Features co-occur, so this is a descriptive inventory, not a feature support score."""
    queries = _index(feature_report, is_feature=True)
    executions = _index(execution_report, is_feature=False)
    grouped = defaultdict(list)
    for number, record in sorted(queries.items()):
        for feature in record["features"]:
            grouped[feature].append(number)
    results = []
    for feature, numbers in sorted(grouped.items()):
        verified = [n for n in numbers if executions[n]["status"] == "passed"]
        unverified = [n for n in numbers if executions[n]["status"] == "unverified"]
        failed = [n for n in numbers if executions[n]["status"] not in ("passed", "unverified")]
        results.append({
            "feature": feature,
            "query_count": len(numbers),
            "verified_count": len(verified),
            "unverified_count": len(unverified),
            "failed_count": len(failed),
            "queries": numbers,
            "failed_queries": failed,
        })
    return {
        "workload": "tpch-feature-execution-matrix",
        "official_tpch_score": False,
        "feature_support_inferred": False,
        "total_queries": 22,
        "features": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--features", type=Path, required=True)
    parser.add_argument("--execution", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = summarize(load_report(args.features), load_report(args.execution))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, f"TPC-H feature matrix failed: {error}\n")
    payload = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

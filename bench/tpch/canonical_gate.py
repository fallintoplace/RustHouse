#!/usr/bin/env python3
"""Require independent reference-verified success for all 22 TPC-H queries."""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from bench.tpch.canonical_integrity import verify
from bench.tpch.run import load_queries, query_fingerprint

EXPECTED = set(range(1, 23))


def trusted_fingerprint():
    """Hash the verified pinned full SQL, not a caller-selected query folder."""
    query_dir = Path(__file__).resolve().parent / "queries"
    verify(query_dir)
    return query_fingerprint(load_queries(query_dir))


def audit(report):
    if not isinstance(report, dict):
        raise ValueError("benchmark report must be a JSON object")
    if report.get("reference_checked") is not True:
        raise ValueError("all queries need independent expected TSV references")
    if report.get("query_sha256") != trusted_fingerprint():
        raise ValueError("benchmark did not run the pinned full canonical TPC-H SQL")
    records = report.get("results")
    if not isinstance(records, list) or len(records) != 22:
        raise ValueError("report must contain exactly 22 query results")
    observed = [item.get("query") for item in records if isinstance(item, dict)]
    if len(observed) != 22 or any(type(q) is not int for q in observed):
        raise ValueError("each query result must contain an integer ID")
    if set(observed) != EXPECTED or len(set(observed)) != 22:
        raise ValueError("report must cover each canonical query ID exactly once")
    failures = sorted((item["query"], item.get("status"))
                      for item in records if item.get("status") != "passed")
    if failures:
        raise ValueError(f"full TPC-H verification incomplete: {failures}")
    coverage = report.get("coverage")
    if not isinstance(coverage, dict) or coverage.get("verified_queries") != 22:
        raise ValueError("coverage summary does not attest 22 verified queries")
    if coverage.get("failed_queries") != 0 or coverage.get("unverified_queries") != 0:
        raise ValueError("coverage summary contains failures or unverified results")
    inventory = report.get("official_22_coverage")
    if inventory is not None and not isinstance(inventory, dict):
        raise ValueError("SQL corpus inventory must be an object")
    if inventory is not None and (
        inventory.get("available_queries") != 22
        or inventory.get("missing_query_ids")
        or inventory.get("unexpected_query_ids")
    ):
        raise ValueError("SQL corpus inventory is incomplete")
    return {"passed": True, "reference_verified_queries": 22,
            "canonical_query_ids": list(range(1, 23)),
            "official_tpch_score": False}



MAX_REPORT_BYTES = 16 * 1024 * 1024


def _read_bounded_report(path):
    with path.open("rb") as source:
        contents = source.read(MAX_REPORT_BYTES + 1)
    if len(contents) > MAX_REPORT_BYTES:
        raise ValueError("benchmark JSON report exceeds 16 MiB")
    return contents.decode("utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = audit(json.loads(_read_bounded_report(args.report)))
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        parser.exit(1, f"TPC-H 22/22 correctness gate failed: {error}\n")
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output is not None:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

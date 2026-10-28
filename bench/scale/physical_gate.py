"""Gate persisted-data COUNT and SUM results using independent references.

Do not use for synthetic numbers() timing, metadata-only COUNT, or official
cross-engine benchmark claims. Targets are user-supplied, not CI defaults.
"""
import argparse
import json
import math
from pathlib import Path

from bench.scale.physical_scan import COUNT_SQL, SUM_SQL, even_reference

QUERIES = ("count_even", "sum_even")


def validate(report, *, min_rows=1_000_000_000, max_median_seconds=None,
             max_p90_seconds=None):
    if min_rows < 1:
        raise ValueError("min_rows must be positive")
    for value in (max_median_seconds, max_p90_seconds):
        if value is not None and (not math.isfinite(value) or value <= 0):
            raise ValueError("timing targets must be positive and finite")
    if report.get("workload") != "rusthouse-persisted-wide-part-full-scans":
        raise ValueError("wrong workload: expected physical persisted scan")
    rows = report.get("rows_written_and_verified")
    if type(rows) is not int or rows < min_rows:
        raise ValueError("not enough verified persisted rows")
    count, total = even_reference(rows)
    expected = {"count_even": (COUNT_SQL, count), "sum_even": (SUM_SQL, total)}
    checks = []
    for query in QUERIES:
        result = report.get(query)
        if not isinstance(result, dict) or result.get("verified") is not True:
            raise ValueError(f"{query}: missing verified oracle match")
        expected_sql, expected_value = expected[query]
        if result.get("query") != expected_sql or result.get("expected") != expected_value:
            raise ValueError(f"{query}: query or expected oracle mismatch")
        median, p90 = result.get("median_seconds"), result.get("p90_seconds")
        if not all(type(t) in (float, int) and math.isfinite(t) and t > 0 for t in (median, p90)):
            raise ValueError(f"{query}: invalid timings")
        if p90 < median:
            raise ValueError(f"{query}: p90 must not be below median")
        checks.append({"query": query, "median_seconds": median,
                       "p90_seconds": p90,
                       "within_target": (max_median_seconds is None or median <= max_median_seconds)
                       and (max_p90_seconds is None or p90 <= max_p90_seconds)})
    return {"workload": "verified-physical-scan-gate", "rows": rows,
            "all_passed": all(check["within_target"] for check in checks),
            "checks": checks}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--min-rows", type=int, default=1_000_000_000)
    parser.add_argument("--max-median-seconds", type=float)
    parser.add_argument("--max-p90-seconds", type=float)
    args = parser.parse_args(argv)
    try:
        report = validate(json.loads(args.report.read_text(encoding="utf-8")),
                          min_rows=args.min_rows,
                          max_median_seconds=args.max_median_seconds,
                          max_p90_seconds=args.max_p90_seconds)
    except (OSError, ValueError) as error:
        parser.exit(1, f"physical gate failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0 if report["all_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

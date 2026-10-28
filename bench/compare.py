#!/usr/bin/env python3
"""Compare reference-checked SQL benchmark JSON reports query by query.

Only compare data with matching engine, workload, scale, and timing scope.
"""
import argparse
import json
import math
from pathlib import Path


VALID = {"passed", "unverified"}


def index(results):
    if not isinstance(results, list) or not results:
        raise ValueError("report requires nonempty query results")
    indexed = {}
    for record in results:
        if not isinstance(record, dict):
            raise ValueError("query results must be objects")
        ident = record.get("query")
        if type(ident) is not int or ident < 1 or ident in indexed:
            raise ValueError("query IDs must be distinct positive integers")
        indexed[ident] = record
    return indexed


def elapsed(record):
    latency = record.get("latency")
    if not isinstance(latency, dict):
        return None
    value = latency.get("median_seconds")
    if type(value) not in (float, int) or not math.isfinite(value) or value <= 0:
        return None
    return value


def ensure_compatible_reports(baseline, candidate):
    for field in ("workload", "engine", "rows", "seed", "timing_scope"):
        old = baseline.get(field)
        new = candidate.get(field)
        if old is not None and new is not None and old != new:
            raise ValueError(f"cannot compare different {field} values")


def compare(baseline, candidate, slowdown_threshold_percent=10):
    if not math.isfinite(slowdown_threshold_percent) or slowdown_threshold_percent < 0:
        raise ValueError("slowdown threshold must be nonnegative and finite")
    ensure_compatible_reports(baseline, candidate)
    left = index(baseline.get("results"))
    right = index(candidate.get("results"))
    if left.keys() != right.keys():
        raise ValueError("query ID sets must match")
    regressions = []
    improvements = []
    incomparable = []
    compared = 0
    for ident in sorted(left):
        old, new = left[ident], right[ident]
        before, after = old.get("status"), new.get("status")
        if before == "passed" and after != "passed":
            regressions.append({"query": ident, "reason": "correctness_downgrade",
                                "baseline_status": before, "candidate_status": after})
            continue
        if before not in VALID or after not in VALID:
            incomparable.append({"query": ident, "reason": "query_not_successful",
                                 "baseline_status": before, "candidate_status": after})
            continue
        previous, current = elapsed(old), elapsed(new)
        if previous is None or current is None:
            incomparable.append({"query": ident, "reason": "missing_positive_median"})
            continue
        compared += 1
        change = 100 * (current / previous - 1)
        item = {"query": ident, "baseline_ms": round(previous * 1000, 6),
                "candidate_ms": round(current * 1000, 6),
                "change_percent": round(change, 3)}
        if change > slowdown_threshold_percent:
            regressions.append({**item, "reason": "latency_regression"})
        elif change < -slowdown_threshold_percent:
            improvements.append(item)
    return {
        "comparison": "sql-query-latency",
        "threshold_percent": slowdown_threshold_percent,
        "queries": len(left),
        "compared_queries": compared,
        "regression_count": len(regressions),
        "regressions": regressions,
        "improvements": improvements,
        "incomparable": incomparable,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--slowdown-threshold-percent", type=float, default=10)
    parser.add_argument("--fail-on-regressions", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = compare(
            json.loads(args.baseline.read_text(encoding="utf-8")),
            json.loads(args.candidate.read_text(encoding="utf-8")),
            args.slowdown_threshold_percent,
        )
    except (OSError, ValueError) as error:
        parser.exit(1, f"benchmark comparison failed: {error}\n")
    content = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(content, encoding="utf-8")
    print(content, end="")
    return int(args.fail_on_regressions and result["regression_count"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())

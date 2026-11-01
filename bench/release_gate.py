#!/usr/bin/env python3
"""Fail release qualification if any required SQL benchmark result is unverified."""
import argparse
import json
import math
import sys
from pathlib import Path

MAX_MANIFEST_BYTES = 2 * 1024 * 1024


def reject_duplicate_keys(pairs):
    data = {}
    for key, value in pairs:
        if key in data:
            raise ValueError(f"duplicate benchmark manifest key: {key}")
        data[key] = value
    return data


def reject_nonstandard_json_number(token):
    raise ValueError(f"nonstandard release manifest JSON number: {token}")


def load_manifest(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("manifest must be a regular non-symlinked file")
    with path.open("rb") as source:
        payload = source.read(MAX_MANIFEST_BYTES + 1)
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds 2 MiB")
    return json.loads(payload, object_pairs_hook=reject_duplicate_keys,
                      parse_constant=reject_nonstandard_json_number)



def parse_query_ids(spec):
    ids = set()
    for fragment in spec.split(","):
        bounds = fragment.strip().split("-")
        if len(bounds) not in (1, 2) or any(
            not value.isascii() or not value.isdecimal() for value in bounds
        ):
            raise ValueError(f"invalid query ID or range: {fragment!r}")
        start = int(bounds[0])
        end = int(bounds[-1])
        if start < 1 or end < start:
            raise ValueError(f"invalid query ID range: {fragment!r}")
        if end - start > 10_000:
            raise ValueError("query ID range exceeds 10000")
        for number in range(start, end + 1):
            if number in ids:
                raise ValueError(f"duplicate required query ID: {number}")
            ids.add(number)
            if len(ids) > 10_000:
                raise ValueError("required query ID set exceeds 10000")
    if not ids:
        raise ValueError("at least one required query ID is needed")
    return ids


def validate_samples(samples, runs, number):
    if not isinstance(samples, list) or len(samples) != runs:
        raise ValueError(f"query {number} has incomplete timing samples")
    if any(
        type(value) not in (int, float)
        or not math.isfinite(value)
        or value < 0
        for value in samples
    ):
        raise ValueError(f"query {number} has invalid timing samples")


def verify(report, required, expected_revision=None, *, min_runs=1):
    if not isinstance(report, dict) or report.get("reference_checked") is not True:
        raise ValueError("benchmark report must confirm reference comparisons")
    if expected_revision is not None:
        revision = report.get("revision")
        if not expected_revision or not isinstance(revision, str) or revision != expected_revision:
            raise ValueError("benchmark revision differs from the required commit")
    runs = report.get("runs")
    if type(min_runs) is not int or not 1 <= min_runs <= 1000:
        raise ValueError("min_runs must be between 1 and 1000")
    if type(runs) is not int or runs < min_runs:
        raise ValueError("benchmark report has fewer than required timing runs")
    rows = report.get("results")
    if not isinstance(rows, list) or not rows:
        raise ValueError("benchmark report contains no query results")
    seen = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("invalid query result")
        number = row.get("query")
        if type(number) is not int or number < 1:
            raise ValueError("invalid query result ID")
        if number in seen:
            raise ValueError(f"duplicate query result: {number}")
        seen.add(number)
        if row.get("status") != "passed":
            raise ValueError(f"query {number} was not reference-verified: {row.get('status')}")
        samples = row.get("timings_seconds", row.get("seconds"))
        validate_samples(samples, runs, number)
    if seen != required:
        missing = sorted(required - seen)
        unexpected = sorted(seen - required)
        raise ValueError(f"query set mismatch: missing={missing}, unexpected={unexpected}")
    return len(seen)


def safe_median(samples):
    ordered = sorted(samples)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[middle]
    return ordered[middle - 1] + (ordered[middle] - ordered[middle - 1]) / 2


def verify_latency_baseline(candidate, baseline, required, max_slowdown=1.25):
    """Reject median latency regressions after both reports pass correctness checks."""
    if (type(max_slowdown) not in (int, float) or
            not math.isfinite(max_slowdown) or max_slowdown < 1):
        raise ValueError("max slowdown must be a finite ratio >= 1")
    verify(candidate, required)
    verify(baseline, required)
    for field in ("workload", "engine", "rows", "seed", "timing_scope",
                  "query_sha256", "comparison", "distribution", "suite",
                  "source", "projection_manifest", "official_benchmark"):
        old, new = baseline.get(field), candidate.get(field)
        if type(old) is not type(new) or old != new:
            raise ValueError(f"cannot compare different {field} values")
    prior = {row["query"]: row for row in baseline["results"]}
    for row in candidate["results"]:
        number = row["query"]
        old = prior[number]
        old_samples = old.get("timings_seconds", old.get("seconds"))
        new_samples = row.get("timings_seconds", row.get("seconds"))
        old_median = safe_median(old_samples)
        new_median = safe_median(new_samples)
        if (old_median == 0 and new_median > 0) or (
                old_median > 0 and new_median / old_median > max_slowdown):
            raise ValueError(f"query {number} exceeds median latency slowdown limit")



def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="JSON emitted by a RustHouse benchmark runner")
    parser.add_argument(
        "--expected-revision",
        help="Require an exact Git commit SHA in the benchmark report",
    )
    parser.add_argument(
        "--queries", required=True,
        help="required query IDs, e.g. 1-43 or 1,6,12",
    )
    parser.add_argument("--min-runs", type=int, default=1,
                        help="Require a minimum number of timing repetitions")
    parser.add_argument("--baseline-manifest", type=Path,
                        help="Compare query latencies against a verified baseline")
    parser.add_argument("--max-median-slowdown", type=float, default=1.25,
                        help="Maximum allowed candidate/baseline median ratio")
    args = parser.parse_args(argv)
    try:
        required = parse_query_ids(args.queries)
        report = load_manifest(args.report)
        count = verify(report, required, args.expected_revision, min_runs=args.min_runs)
        if args.baseline_manifest is not None:
            baseline = load_manifest(args.baseline_manifest)
            verify_latency_baseline(report, baseline, required, args.max_median_slowdown)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"release SQL gate failed: {error}", file=sys.stderr)
        return 1
    print(f"release SQL gate passed: {count} reference-verified queries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

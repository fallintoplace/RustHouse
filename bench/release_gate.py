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


def load_manifest(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("manifest must be a regular non-symlinked file")
    with path.open("rb") as source:
        payload = source.read(MAX_MANIFEST_BYTES + 1)
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("manifest exceeds 2 MiB")
    return json.loads(payload, object_pairs_hook=reject_duplicate_keys)



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
    args = parser.parse_args(argv)
    try:
        required = parse_query_ids(args.queries)
        report = load_manifest(args.report)
        count = verify(report, required, args.expected_revision, min_runs=args.min_runs)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"release SQL gate failed: {error}", file=sys.stderr)
        return 1
    print(f"release SQL gate passed: {count} reference-verified queries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

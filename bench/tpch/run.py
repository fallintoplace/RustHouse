#!/usr/bin/env python3
"""Run SQL compatibility and end-to-end latency checks against RustHouse."""
import argparse
from collections import Counter
import hashlib
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary
from canonical_integrity import verify as verify_canonical_sql

QUERY_NAME = re.compile(r"q(\d{1,2})\.sql$", re.IGNORECASE)

def require_canonical_corpus(directory):
    """Reject any SQL that differs from pinned upstream full TPC-H Q1-Q22."""
    return verify_canonical_sql(directory)


def load_queries(directory):
    queries = []
    for path in directory.iterdir():
        if not path.is_file():
            continue
        match = QUERY_NAME.fullmatch(path.name)
        if match is None:
            continue
        number = int(match.group(1))
        if number < 1:
            raise ValueError(f"query IDs must be positive: {path}")
        sql = "\n".join(
            line for line in path.read_text(encoding="utf-8").splitlines()
            if not line.lstrip().startswith("--")
        ).strip().rstrip(";").strip()
        if not sql:
            raise ValueError(f"empty SQL query: {path}")
        queries.append((number, sql))
    queries.sort(key=lambda item: item[0])
    if not queries:
        raise ValueError(f"no qNN.sql files in {directory}")
    if len({number for number, _ in queries}) != len(queries):
        raise ValueError("duplicate query numbers")
    return queries

def query_fingerprint(queries):
    digest = hashlib.sha256()
    for number, sql in sorted(queries):
        digest.update(f"q{number:02d}\0".encode("ascii"))
        digest.update(sql.encode("utf-8"))
        digest.update(b"\0")
    return digest.hexdigest()


def invoke(binary, database, sql, timeout):
    return cli_sql(binary, database, sql, timeout)


def summary(seconds):
    return latency_summary(seconds)

def compare_reference(actual, reference, mode="exact"):
    if mode == "exact":
        return actual == reference
    if mode == "unordered":
        return Counter(actual.splitlines(keepends=True)) == Counter(
            reference.splitlines(keepends=True)
        )
    raise ValueError(f"unknown reference comparison mode: {mode}")


def benchmark_one(number, sql, execute, runs, warmups, reference=None, comparison="exact"):
    elapsed = []
    status = "unverified" if reference is None else "passed"
    error = None
    for iteration in range(-warmups, runs):
        started = time.perf_counter() if iteration >= 0 else None
        try:
            output = execute(sql)
        except subprocess.TimeoutExpired:
            status, error = "timeout", "SQL execution timed out"
            break
        except (OSError, RuntimeError, ValueError) as exc:
            status, error = "error", str(exc)
            break
        # Stop the timer before expensive reference and multiset comparison.
        finished = time.perf_counter() if started is not None else None
        if reference is not None and not compare_reference(output, reference, comparison):
            status, error = "mismatch", "output differs from reference bytes"
            break
        if finished is not None:
            elapsed.append(finished - started)
    # Do not publish incomplete latency distributions for failed queries.
    if status not in ("passed", "unverified"):
        elapsed = []
    return {
        "query": number, "status": status,
        "timings_seconds": elapsed, "latency": summary(elapsed),
        "error": error,
    }

def summarize_coverage(results):
    total = len(results)
    verified = sum(result["status"] == "passed" for result in results)
    unverified = sum(result["status"] == "unverified" for result in results)
    executed = verified + unverified
    return {
        "total_queries": total,
        "verified_queries": verified,
        "unverified_queries": unverified,
        "failed_queries": total - executed,
        "execution_success_percent": round(100 * executed / total, 2) if total else 0.0,
        "verified_correct_percent": round(100 * verified / total, 2) if total else 0.0,
    }


def official_22_coverage(queries):
    present = {number for number, _ in queries}
    expected = set(range(1, 23))
    return {
        "available_queries": len(present & expected),
        "missing_query_ids": sorted(expected - present),
        "unexpected_query_ids": sorted(present - expected),
        "coverage_percent": round(100 * len(present & expected) / 22, 2),
    }


def select_queries(queries, spec):
    if spec is None:
        return queries
    selected = set()
    try:
        for fragment in spec.split(","):
            parts = fragment.strip().split("-")
            if not 1 <= len(parts) <= 2 or not all(
                part.isascii() and part.isdecimal() for part in parts
            ):
                raise ValueError("invalid query ID or range")
            first = int(parts[0])
            last = int(parts[-1])
            if first < 1 or last < first or last > 99:
                raise ValueError("query IDs must be in increasing order within 1..99")
            selected.update(range(first, last + 1))
    except ValueError as exc:
        raise ValueError("--only must contain query IDs or ranges such as 1,6,10-12") from exc
    available = {number for number, _ in queries}
    if not selected or not selected.issubset(available):
        raise ValueError("--only contains an unavailable query ID")
    return [(number, sql) for number, sql in queries if number in selected]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, required=True)
    parser.add_argument("--reference-dir", type=Path)
    parser.add_argument("--comparison", choices=("exact", "unordered"), default="exact",
                        help="Compare row order exactly or as a multiset")
    parser.add_argument("--require-verified", action="store_true",
                        help="Require golden output for every measured query")
    parser.add_argument("--only", help="Comma-separated query IDs, e.g. 1,6,12")
    parser.add_argument("--require-22", action="store_true",
                        help="Reject query directories missing any of the 22 official IDs")
    parser.add_argument("--require-canonical", action="store_true",
                        help="Reject modified, simplified or missing upstream Q1-Q22 SQL")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--expected-lineitem-rows", type=int)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.require_verified and args.reference_dir is None:
        parser.error("--require-verified needs --reference-dir")
    if args.runs < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error("runs and timeout must be positive; warmups cannot be negative")
    if args.expected_lineitem_rows is not None and args.expected_lineitem_rows < 0:
        parser.error("expected row count must be non-negative")
    try:
        if args.require_canonical:
            require_canonical_corpus(args.queries_dir)
        queries = load_queries(args.queries_dir)
    except (OSError, ValueError) as exc:
        parser.error(str(exc))
    try:
        official_coverage = official_22_coverage(queries)
        if args.require_22 and (official_coverage["missing_query_ids"]
                                or official_coverage["unexpected_query_ids"]):
            parser.error("query directory must contain exactly q01.sql to q22.sql")
        queries = select_queries(queries, args.only)
    except ValueError as exc:
        parser.error(str(exc))

    execute = lambda sql: invoke(args.binary, args.database, sql, args.timeout)
    if args.expected_lineitem_rows is not None:
        try:
            actual = execute("SELECT count(*) FROM lineitem").decode("utf-8").strip()
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
            parser.exit(1, f"row-count preflight failed: {exc}\n")
        if actual != str(args.expected_lineitem_rows):
            parser.exit(
                1, f"row-count preflight: expected {args.expected_lineitem_rows}, got {actual}\n"
            )
    results = []
    for number, sql in queries:
        reference = None
        if args.reference_dir is not None:
            path = args.reference_dir / f"q{number:02}.tsv"
            if not path.is_file():
                results.append({
                    "query": number, "status": "missing_reference",
                    "timings_seconds": [], "latency": None, "error": str(path),
                })
                continue
            reference = path.read_bytes()
        results.append(benchmark_one(
            number, sql, execute, args.runs, args.warmups, reference,
            comparison=args.comparison,
        ))
    report = {
        "engine": "rusthouse-cli",
        "timing_scope": "end-to-end CLI invocation, excluding query load and preflight",
        "runs": args.runs, "warmups": args.warmups,
        "query_sha256": query_fingerprint(queries),
        "reference_checked": args.reference_dir is not None,
        "comparison": args.comparison,
        "coverage": summarize_coverage(results),
        "official_22_coverage": official_coverage,
        "results": results,
    }
    emit_json(report, args.output)
    if args.require_verified:
        return int(any(result["status"] != "passed" for result in results))
    return int(any(result["status"] not in ("passed", "unverified") for result in results))

if __name__ == "__main__":
    raise SystemExit(main())

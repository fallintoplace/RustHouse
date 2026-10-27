#!/usr/bin/env python3
"""Run SQL compatibility and end-to-end latency checks against RustHouse."""
import argparse
import hashlib
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary

QUERY_NAME = re.compile(r"q(\d{1,2})\.sql$", re.IGNORECASE)

def load_queries(directory):
    queries = []
    for path in directory.glob("q*.sql"):
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
        digest.update(f"q{number:02d}\\0".encode("ascii"))
        digest.update(sql.encode("utf-8"))
        digest.update(b"\\0")
    return digest.hexdigest()


def invoke(binary, database, sql, timeout):
    return cli_sql(binary, database, sql, timeout)


def summary(seconds):
    return latency_summary(seconds)

def benchmark_one(number, sql, execute, runs, warmups, reference=None):
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
        if reference is not None and output != reference:
            status, error = "mismatch", "output differs from reference bytes"
            break
        if started is not None:
            elapsed.append(time.perf_counter() - started)
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


def select_queries(queries, spec):
    if not spec:
        return queries
    try:
        selected = {int(fragment) for fragment in spec.split(",")}
    except ValueError as exc:
        raise ValueError("--only must contain comma-separated integer IDs") from exc
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
    parser.add_argument("--only", help="Comma-separated query IDs, e.g. 1,6,12")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--expected-lineitem-rows", type=int)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.runs < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error("runs and timeout must be positive; warmups cannot be negative")
    if args.expected_lineitem_rows is not None and args.expected_lineitem_rows < 0:
        parser.error("expected row count must be non-negative")
    try:
        queries = load_queries(args.queries_dir)
    except (OSError, ValueError) as exc:
        parser.error(str(exc))
    try:
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
            number, sql, execute, args.runs, args.warmups, reference
        ))
    report = {
        "engine": "rusthouse-cli",
        "timing_scope": "end-to-end CLI invocation, excluding query load and preflight",
        "runs": args.runs, "warmups": args.warmups,
        "query_sha256": query_fingerprint(queries),
        "reference_checked": args.reference_dir is not None,
        "coverage": summarize_coverage(results),
        "results": results,
    }
    emit_json(report, args.output)
    return int(any(result["status"] not in ("passed", "unverified") for result in results))

if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Benchmark official ClickBench queries 1-9 against synthetic supported columns.

Each query is verified against SQLite before its sample is accepted.
This is a subset, NOT a full 43-query official ClickBench score.
"""
import argparse
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary
from run import DEFAULT_QUERIES, load_queries
from sqlite_oracle import assert_matches, sqlite_query

MAX_QUERY = 9


def parse_ids(spec):
    if not spec:
        return list(range(1, MAX_QUERY + 1))
    ids = set()
    for fragment in spec.split(","):
        pair = fragment.split("-", 1)
        try:
            start = int(pair[0])
            stop = int(pair[1]) if len(pair) == 2 else start
        except ValueError as error:
            raise ValueError("--only accepts numeric IDs and inclusive ranges") from error
        if not 1 <= start <= stop <= MAX_QUERY:
            raise ValueError(f"query IDs must be within 1..{MAX_QUERY}")
        ids.update(range(start, stop + 1))
    return sorted(ids)


def run_subset(binary, database, sqlite_db, only=None, runs=3, warmups=1,
               timeout=120, execute=None):
    if runs < 1 or warmups < 0 or timeout <= 0:
        raise ValueError("invalid timing parameters")
    queries = load_queries(DEFAULT_QUERIES)
    if len(queries) != 43:
        raise ValueError("expected pinned 43-query ClickBench workload")
    ids = parse_ids(only)
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    results = []
    for number in ids:
        sql = queries[number - 1]
        try:
            expected = sqlite_query(sqlite_db, sql)
        except sqlite3.Error as error:
            results.append({"query": number, "status": "reference_error",
                            "error": str(error), "timings_seconds": [], "latency": None})
            continue
        samples = []
        status, failure = "passed", None
        for iteration in range(-warmups, runs):
            started = time.perf_counter() if iteration >= 0 else None
            try:
                output = execute(sql)
                finished = time.perf_counter() if started is not None else None
                assert_matches(output, expected, unordered=(number in (8, 9)))
            except subprocess.TimeoutExpired:
                status, failure = "timeout", "RustHouse query timed out"
                break
            except ValueError as error:
                status, failure = "mismatch", str(error)
                break
            except (OSError, RuntimeError) as error:
                status, failure = "error", str(error)
                break
            if started is not None:
                samples.append(finished - started)
        if status != "passed":
            samples = []
        results.append({"query": number, "status": status, "error": failure,
                        "timings_seconds": samples, "latency": latency_summary(samples)})
    verified = sum(result["status"] == "passed" for result in results)
    return {"workload": "clickbench-queries-1-9-synthetic",
            "official_clickbench_score": False, "attempted_queries": len(results),
            "verified_queries": verified, "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--sqlite-db", type=Path, required=True)
    parser.add_argument("--only", help="Subset of IDs 1..9, e.g. 1-4,7")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_subset(args.binary, args.database, args.sqlite_db, args.only,
                            args.runs, args.warmups, args.timeout)
    except (OSError, ValueError) as error:
        parser.exit(1, f"ClickBench subset failed: {error}\n")
    emit_json(report, args.output)
    return int(report["verified_queries"] != report["attempted_queries"])


if __name__ == "__main__":
    raise SystemExit(main())

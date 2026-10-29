#!/usr/bin/env python3
"""Attempt all 22 supplied qNN.sql queries and report exact execution coverage.

A successful query without a trusted reference is UNVERIFIED. This does not
calculate a TPC-H Power, Throughput, or QphH score.
"""
import argparse
import math
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from run import benchmark_one, summarize_coverage, query_fingerprint

MAX_REFERENCE_BYTES = 64 * 1024 * 1024


def clean_sql(path):
    text = "\n".join(line for line in path.read_text(encoding="utf-8").splitlines()
                     if not line.lstrip().startswith("--")).strip()
    return text.rstrip(";").strip()


def coverage(binary, database, query_dir, reference_dir=None, runs=3, warmups=1,
             timeout=120, execute=None):
    if (type(runs) is not int or type(warmups) is not int
            or runs < 1 or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("invalid timing parameters")
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    results = []
    executed_sql = []
    for number in range(1, 23):
        path = Path(query_dir) / f"q{number:02d}.sql"
        if not path.is_file():
            results.append({"query": number, "status": "missing_query",
                            "timings_seconds": [], "latency": None, "error": str(path)})
            continue
        sql = clean_sql(path)
        if not sql or len(sql.encode("utf-8")) > 64 * 1024:
            results.append({"query": number, "status": "unsupported_query",
                            "timings_seconds": [], "latency": None,
                            "error": "empty query or exceeds SQL statement limit"})
            continue
        executed_sql.append((number, sql))
        reference = None
        if reference_dir is not None:
            ref = Path(reference_dir) / f"q{number:02d}.tsv"
            if not ref.is_file():
                results.append({"query": number, "status": "missing_reference",
                                "timings_seconds": [], "latency": None, "error": str(ref)})
                continue
            # Do not buffer unbounded or symlinked third-party reference files.
            if ref.is_symlink() or ref.stat().st_size > MAX_REFERENCE_BYTES:
                results[number - 1] = {"query": number, "status": "reference_error",
                                        "timings_seconds": [], "latency": None,
                                        "error": "unsafe or oversized reference file"}
                continue
            with ref.open("rb") as stream:
                reference = stream.read(MAX_REFERENCE_BYTES + 1)
            if len(reference) > MAX_REFERENCE_BYTES:
                results[number - 1] = {"query": number, "status": "reference_error",
                                        "timings_seconds": [], "latency": None,
                                        "error": "reference file grew beyond byte limit"}
                continue
        results.append(benchmark_one(number, sql, execute, runs, warmups, reference))
    return {"workload": "tpch-22-query-execution-inventory",
            "query_sha256": query_fingerprint(executed_sql),
            "official_tpch_score": False, "reference_checked": reference_dir is not None,
            "coverage": summarize_coverage(results), "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, required=True)
    parser.add_argument("--reference-dir", type=Path)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = coverage(args.binary, args.database, args.queries_dir,
                          args.reference_dir, args.runs, args.warmups, args.timeout)
    except (OSError, ValueError) as error:
        parser.exit(1, f"TPC-H inventory failed: {error}\n")
    emit_json(result, args.output)
    return int(result["coverage"]["failed_queries"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Attempt all 22 supplied qNN.sql queries and report exact execution coverage.

A successful query without a trusted reference is UNVERIFIED. This does not
calculate a TPC-H Power, Throughput, or QphH score.
"""
import argparse
import math
import os
import stat
from concurrent.futures import ThreadPoolExecutor
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, file_change_time_ns
from run import benchmark_one, summarize_coverage, query_fingerprint

MAX_REFERENCE_BYTES = 64 * 1024 * 1024
MAX_QUERY_FILE_BYTES = 64 * 1024
TPCH_QUERY_COUNT = 22


def _read_stable_file(path, limit, label, limit_label):
    """Pin a regular input inode while reading bounded SQL or reference bytes."""
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError(f"{label} must be regular and non-symlinked")

    def identity(info):
        return (info.st_dev, info.st_ino, info.st_size,
                info.st_mtime_ns, file_change_time_ns(info))

    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or identity(before) != identity(opened):
            raise ValueError(f"{label} changed while opening")
        payload = source.read(limit + 1)
        if identity(opened) != identity(os.fstat(source.fileno())):
            raise ValueError(f"{label} changed during reading")
    if identity(before) != identity(path.lstat()):
        raise ValueError(f"{label} changed during reading")
    if len(payload) > limit:
        raise ValueError(f"{label} exceeds {limit_label}")
    return payload


def clean_sql(path):
    payload = _read_stable_file(path, MAX_QUERY_FILE_BYTES,
                                "TPC-H SQL source", "64 KiB")
    lines = payload.decode("utf-8").splitlines()
    # Only discard provenance comments before the SQL statement. A line starting
    # with '--' inside a multi-line quoted literal is query data, not a comment.
    start = 0
    while start < len(lines) and (
        not lines[start].strip() or lines[start].lstrip().startswith("--")
    ):
        start += 1
    text = "\n".join(lines[start:]).strip()
    return text.rstrip(";").strip()

def coverage(binary, database, query_dir, reference_dir=None, runs=3, warmups=1,
             timeout=120, execute=None, jobs=1):
    if (type(runs) is not int or type(warmups) is not int
            or runs < 1 or warmups < 0
            or type(timeout) not in (int, float)
            or not math.isfinite(timeout) or timeout <= 0
            or type(jobs) is not int or not 1 <= jobs <= TPCH_QUERY_COUNT):
        raise ValueError("invalid timing parameters")
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    results = [None] * TPCH_QUERY_COUNT
    executed_sql = []
    pending = []
    for number in range(1, TPCH_QUERY_COUNT + 1):
        path = Path(query_dir) / f"q{number:02d}.sql"
        if not path.is_file():
            results[number - 1] = {"query": number, "status": "missing_query",
                            "timings_seconds": [], "latency": None, "error": str(path)}
            continue
        try:
            sql = clean_sql(path)
        except (OSError, UnicodeError, ValueError) as error:
            results[number - 1] = {"query": number, "status": "unsupported_query",
                                    "timings_seconds": [], "latency": None,
                                    "error": str(error)[:200]}
            continue
        if not sql or len(sql.encode("utf-8")) > 64 * 1024:
            results[number - 1] = {"query": number, "status": "unsupported_query",
                            "timings_seconds": [], "latency": None,
                            "error": "empty query or exceeds SQL statement limit"}
            continue
        executed_sql.append((number, sql))
        ref = None
        if reference_dir is not None:
            ref = Path(reference_dir) / f"q{number:02d}.tsv"
            if not ref.is_file():
                results[number - 1] = {"query": number, "status": "missing_reference",
                                "timings_seconds": [], "latency": None, "error": str(ref)}
                continue
            # Only retain paths during preflight. Read references inside workers
            # so peak retained reference memory scales with jobs, not 22 queries.
            if ref.is_symlink() or ref.stat().st_size > MAX_REFERENCE_BYTES:
                results[number - 1] = {"query": number, "status": "reference_error",
                                       "timings_seconds": [], "latency": None,
                                       "error": "unsafe or oversized reference file"}
                continue
        pending.append((number, sql, ref))

    def run_query(item):
        number, sql, ref = item
        reference = None
        if ref is not None:
            try:
                if ref.is_symlink() or ref.stat().st_size > MAX_REFERENCE_BYTES:
                    raise ValueError("unsafe or oversized reference file")
                reference = _read_stable_file(
                    ref, MAX_REFERENCE_BYTES, "reference file", "64 MiB"
                )
            except (OSError, ValueError) as error:
                return {"query": number, "status": "reference_error",
                        "timings_seconds": [], "latency": None,
                        "error": str(error)[:200]}
        return benchmark_one(number, sql, execute, runs, warmups, reference)

    if jobs == 1:
        completed = map(run_query, pending)
        for item in completed:
            results[item["query"] - 1] = item
    else:
        with ThreadPoolExecutor(max_workers=jobs) as pool:
            for item in pool.map(run_query, pending):
                results[item["query"] - 1] = item
    return {"workload": "tpch-22-query-execution-inventory",
            "query_sha256": query_fingerprint(executed_sql),
            "official_tpch_score": False, "reference_checked": reference_dir is not None,
            "concurrent_queries": jobs, "timing_scope": "contended CLI queries" if jobs > 1 else "serial CLI queries",
            "coverage": summarize_coverage(results), "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, required=True)
    parser.add_argument("--reference-dir", type=Path)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=1,
                        help="Parallel query inventory workers; use 1 for comparable latency")
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = coverage(args.binary, args.database, args.queries_dir,
                          args.reference_dir, args.runs, args.warmups, args.timeout,
                          jobs=args.jobs)
    except (OSError, ValueError) as error:
        parser.exit(1, f"TPC-H inventory failed: {error}\n")
    emit_json(result, args.output)
    return int(result["coverage"]["failed_queries"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Measure concurrent CLI SQL throughput over a chosen query set.

This is a stress workload. It is not the official TPC-H Throughput Test.
"""
import argparse
import concurrent.futures
import subprocess
import time
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary
from run import load_queries, select_queries


def run_throughput(queries, execute, clients=2, rounds=2, references=None,
                   require_verified=False):
    if (type(clients) is not int or type(rounds) is not int
            or not 1 <= clients <= 64 or not 1 <= rounds <= 100):
        raise ValueError("clients must be 1..64 and rounds 1..100")
    if not queries:
        raise ValueError("at least one SQL query is required")
    references = references or {}
    if not isinstance(references, dict) or any(
            type(number) is not int or type(golden) is not bytes
            for number, golden in references.items()):
        raise ValueError("throughput references must map integer IDs to bytes")
    if require_verified and any(number not in references for number, _ in queries):
        raise ValueError("verified throughput requires a reference for every query")
    # Explicitly cap the planned jobs to avoid runaway CI load.
    if len(queries) * rounds > 2200:
        raise ValueError("no more than 2200 query executions per run")
    seen = set()
    for entry in queries:
        if (not isinstance(entry, (tuple, list)) or len(entry) != 2
                or type(entry[0]) is not int or entry[0] < 1
                or entry[0] in seen or not isinstance(entry[1], str)
                or not entry[1].strip()):
            raise ValueError("queries need distinct positive IDs and nonempty SQL")
        seen.add(entry[0])
    tasks = [(number, sql) for _ in range(rounds) for number, sql in queries]

    def once(task):
        number, sql = task
        started = time.perf_counter()
        try:
            output = execute(sql)
            if type(output) is not bytes:
                raise TypeError("SQL execution must return bytes")
        except subprocess.TimeoutExpired:
            return {"query": number, "status": "timeout",
                    "seconds": None, "error": "query timed out"}
        except (OSError, RuntimeError, ValueError, TypeError) as error:
            return {"query": number, "status": "error",
                    "seconds": None, "error": str(error)[:1000]}
        duration = time.perf_counter() - started
        expected = references.get(number)
        status = "mismatch" if expected is not None and output != expected else (
            "passed" if expected is not None else "unverified"
        )
        return {"query": number, "status": status,
                "seconds": duration, "error": "result differs from reference" if status == "mismatch" else None}

    started = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=clients) as pool:
        results = list(pool.map(once, tasks))
    elapsed = time.perf_counter() - started
    successful = [r for r in results if r["status"] in ("passed", "unverified")]
    verified = sum(r["status"] == "passed" for r in results)
    durations = [r["seconds"] for r in successful]
    return {
        "workload": "tpch-concurrent-cli-stress",
        "official_tpch_score": False,
        "concurrency": clients, "rounds": rounds,
        "planned_executions": len(tasks),
        "successful_executions": len(successful),
        "verified_executions": verified,
        "all_executions_verified": verified == len(tasks),
        "verification_required": require_verified,
        "elapsed_seconds": elapsed,
        "successful_queries_per_second": len(successful) / elapsed if elapsed > 0 else None,
        "latency": latency_summary(durations),
        "results": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--queries-dir", type=Path, required=True)
    parser.add_argument("--reference-dir", type=Path)
    parser.add_argument("--require-verified", action="store_true",
                        help="Require a reference match for every throughput sample")
    parser.add_argument("--only")
    parser.add_argument("--clients", type=int, default=2)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if args.require_verified and args.reference_dir is None:
        parser.error("--require-verified needs --reference-dir")
    try:
        queries = select_queries(load_queries(args.queries_dir), args.only)
        references = {}
        if args.reference_dir:
            for number, _ in queries:
                path = args.reference_dir / f"q{number:02}.tsv"
                if not path.is_file():
                    raise ValueError(f"missing reference file {path}")
                references[number] = path.read_bytes()
        report = run_throughput(
            queries,
            lambda sql: cli_sql(args.binary, args.database, sql, args.timeout),
            clients=args.clients, rounds=args.rounds, references=references,
            require_verified=args.require_verified,
        )
    except (OSError, ValueError) as error:
        parser.exit(1, f"throughput run failed: {error}\n")
    emit_json(report, args.output)
    return int(report["successful_executions"] != report["planned_executions"]
               or (args.require_verified and not report["all_executions_verified"]))


if __name__ == "__main__":
    raise SystemExit(main())

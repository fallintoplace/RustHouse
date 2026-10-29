#!/usr/bin/env python3
"""Compare verified, cold CLI SQL latency on equivalent RustHouse and ClickHouse data.

RustHouse starts a fresh CLI process while ClickHouse uses a native server client.
The paired ABBA schedule reduces order bias. A missing or mismatching reference invalidates the query.
The measurements include CLI startup, client transport and server cache effects.
This is not an official TPC-H benchmark or a fair in-process throughput test.
"""
import argparse
import os
import hashlib
import json
import math
import statistics
import subprocess
import time
from pathlib import Path

from bench.tpch.run import compare_reference, load_queries, query_fingerprint
from bench.tpch.canonical_integrity import verify as verify_canonical_sql

ENGINES = ("rusthouse", "clickhouse")
ABBA = ("rusthouse", "clickhouse", "clickhouse", "rusthouse")
BAAB = ("clickhouse", "rusthouse", "rusthouse", "clickhouse")
MAX_REFERENCE_BYTES = 16 * 1024 * 1024


def command(engine, binary, database, sql):
    if engine == "rusthouse":
        return [str(binary), "sql", sql, "--database", str(database),
                "--format", "tsv-no-header"]
    if engine == "clickhouse":
        # Use an existing ClickHouse server with the same logical fixture.
        # TabSeparated escapes text and emits headerless TSV like RustHouse.
        return [str(binary), "--database", str(database),
                "--format", "TabSeparated", "--query", sql]
    raise ValueError(f"unknown engine: {engine}")


def invoke(args, timeout):
    start = time.perf_counter()
    result = subprocess.run(args, capture_output=True, check=False, timeout=timeout)
    duration = time.perf_counter() - start
    if result.returncode:
        stderr = result.stderr.decode("utf-8", "replace")
        raise RuntimeError(f"exit {result.returncode}: {stderr[-500:]}")
    return result.stdout, duration


def benchmark_pair(sql, reference, execute, runs=5, warmups=2, comparison="exact"):
    if runs < 1 or warmups < 0:
        raise ValueError("runs must be positive and warmups nonnegative")
    if not isinstance(reference, bytes):
        raise TypeError("reference must be exact TSV bytes")

    samples = {engine: [] for engine in ENGINES}
    def checked(engine):
        output, seconds = execute(engine, sql)
        if not compare_reference(output, reference, comparison):
            raise ValueError(f"{engine} result differs from the reference TSV")
        if not math.isfinite(seconds) or seconds <= 0:
            raise ValueError("timings must be finite and positive")
        return seconds

    for _ in range(warmups):
        for engine in ENGINES:
            checked(engine)
    for round_index in range(runs):
        # Alternate the engine starting first across runs to reduce warm-cache
        # and CPU-frequency order effects without unbalancing sample counts.
        order = ABBA if round_index % 2 == 0 else BAAB
        for engine in order:
            samples[engine].append(checked(engine))

    medians = {name: statistics.median(times) for name, times in samples.items()}
    # Nearest-rank p90 exposes long-tail regressions hidden by median alone.
    p90 = {name: sorted(times)[math.ceil(len(times) * 0.9) - 1]
           for name, times in samples.items()}
    ratio = medians["clickhouse"] / medians["rusthouse"]
    if not math.isfinite(ratio) or ratio <= 0:
        raise ValueError("paired speed ratio is not finite and positive")
    # Tail speedup can disagree with median speedup for spiky workloads.
    p90_ratio = p90["clickhouse"] / p90["rusthouse"]
    if not math.isfinite(p90_ratio) or p90_ratio <= 0:
        raise ValueError("paired p90 speed ratio is not finite and positive")
    return {
        "rusthouse_median_seconds": medians["rusthouse"],
        "clickhouse_median_seconds": medians["clickhouse"],
        "rusthouse_p90_seconds": p90["rusthouse"],
        "clickhouse_p90_seconds": p90["clickhouse"],
        "clickhouse_over_rusthouse": ratio,
        "clickhouse_p90_over_rusthouse": p90_ratio,
        "rusthouse_samples_seconds": samples["rusthouse"],
        "clickhouse_samples_seconds": samples["clickhouse"],
    }


def compare(binary, database, clickhouse_binary, clickhouse_database,
            queries, reference_dir, runs, warmups, timeout, comparison="exact"):
    if timeout <= 0 or not math.isfinite(timeout):
        raise ValueError("timeout must be finite and positive")
    if comparison not in ("exact", "unordered"):
        raise ValueError("comparison must be exact or unordered")
    results = []
    for number, sql in queries:
        ref_path = reference_dir / f"q{number:02}.tsv"
        record = {"query": number, "status": "error",
                  "reference_sha256": None}
        if not ref_path.is_file():
            record["error"] = f"missing reference: {ref_path}"
            results.append(record)
            continue
        if ref_path.is_symlink() or ref_path.stat().st_size > MAX_REFERENCE_BYTES:
            record["error"] = "unsafe or oversized reference TSV"
            results.append(record)
            continue
        with ref_path.open("rb") as source:
            reference = source.read(MAX_REFERENCE_BYTES + 1)
        if len(reference) > MAX_REFERENCE_BYTES:
            record["error"] = "reference exceeds byte limit"
            results.append(record)
            continue
        record["reference_sha256"] = hashlib.sha256(reference).hexdigest()

        def execute(engine, statement):
            parts = command(
                engine,
                binary if engine == "rusthouse" else clickhouse_binary,
                database if engine == "rusthouse" else clickhouse_database,
                statement,
            )
            return invoke(parts, timeout)

        try:
            record.update(benchmark_pair(sql, reference, execute, runs, warmups, comparison))
            record["status"] = "passed"
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
            record["error"] = str(error)[-600:]
        results.append(record)

    all_verified = bool(results) and all(r["status"] == "passed" for r in results)
    ratios = [r["clickhouse_over_rusthouse"] for r in results] if all_verified else []
    return {
        "workload": "paired-clickhouse-cli-sql-latency",
        "revision": os.environ.get("GITHUB_SHA"),
        "timing_scope": "RustHouse fresh CLI vs ClickHouse native TCP client, server cache may be warm",
        "official_tpch_score": False,
        "all_queries_verified": all_verified,
        "comparison": comparison,
        "query_sha256": query_fingerprint(queries),
        "runs": runs, "warmups": warmups,
        "speedup_geomean_clickhouse_over_rusthouse":
            math.exp(sum(math.log(r) for r in ratios) / len(ratios)) if ratios else None,
        "results": results,
    }


def load_paired_queries(directory, require_canonical=False):
    if require_canonical:
        verify_canonical_sql(directory)
    return load_queries(directory)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--clickhouse", default="clickhouse-client")
    parser.add_argument("--clickhouse-database", default="default")
    parser.add_argument("--queries-dir", required=True, type=Path)
    parser.add_argument("--require-canonical", action="store_true",
                        help="Use only exact upstream full Q1-Q22 SQL")
    parser.add_argument("--reference-dir", required=True, type=Path)
    parser.add_argument("--comparison", choices=("exact", "unordered"), default="exact",
                        help="Match ordered rows exactly, or compare unordered row multisets")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        queries = load_paired_queries(args.queries_dir, args.require_canonical)
        report = compare(
            args.binary, args.database, args.clickhouse, args.clickhouse_database,
            queries, args.reference_dir, args.runs, args.warmups, args.timeout,
            comparison=args.comparison,
        )
    except (OSError, ValueError) as error:
        parser.exit(1, f"paired comparison failed: {error}\n")
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["all_queries_verified"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

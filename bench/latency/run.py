#!/usr/bin/env python3
"""Measure verified end-to-end RustHouse CLI latency over 20 generated SQL queries."""

import argparse
import math
import os
import platform
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import emit_json, latency_summary

ROOT = Path(__file__).resolve().parents[2]


@dataclass(frozen=True)
class Case:
    name: str
    sql: str
    expected: bytes


def tsv(values):
    """Encode the independently computed reference rows without a TSV header."""
    return "".join(
        "\t".join(map(str, row if isinstance(row, tuple) else (row,))) + "\n"
        for row in values
    ).encode("utf-8")


def make_cases(rows):
    """Keep results small while increasing the generated input scan size."""
    if not 512 <= rows <= 1_000_000:
        raise ValueError("--rows must be in 512..1000000")
    middle = rows // 2
    multiples_of_7 = range(0, rows, 7)
    multiples_of_10 = range(0, rows, 10)
    last_seven = ((rows - 1) // 7) * 7
    series_multiples_of_21 = range(0, rows, 21)
    return [
        Case("count-all", f"SELECT count(*) FROM numbers({rows})", tsv([rows])),
        Case("count-half", f"SELECT count(*) FROM numbers({rows}) WHERE number >= {middle}",
             tsv([rows - middle])),
        Case("count-mod-7", f"SELECT count(*) FROM numbers({rows}) WHERE number % 7 = 0",
             tsv([len(multiples_of_7)])),
        Case("count-or", f"SELECT count(*) FROM numbers({rows}) WHERE number % 7 = 1 OR number % 7 = 3",
             tsv([len(range(1, rows, 7)) + len(range(3, rows, 7))])),
        Case("count-between", f"SELECT count(*) FROM numbers({rows}) WHERE number BETWEEN 11 AND {rows - 11}",
             tsv([rows - 21])),
        Case("count-in", f"SELECT count(*) FROM numbers({rows}) WHERE number IN (0, 1, 2, 3, 4)",
             tsv([5])),
        Case("sum-half", f"SELECT sum(number) FROM numbers({rows}) WHERE number >= {middle}",
             tsv([sum(range(middle, rows))])),
        Case("sum-mod-7", f"SELECT sum(number) FROM numbers({rows}) WHERE number % 7 = 0",
             tsv([sum(multiples_of_7)])),
        Case("min-all", f"SELECT min(number) FROM numbers({rows})", tsv([0])),
        Case("max-all", f"SELECT max(number) FROM numbers({rows})", tsv([rows - 1])),
        Case("sum-mod-10", f"SELECT sum(number) FROM numbers({rows}) WHERE number % 10 = 0",
             tsv([sum(multiples_of_10)])),
        Case("count-distinct", f"SELECT count(DISTINCT number) FROM numbers({rows})", tsv([rows])),
        Case("filtered-top-desc", f"SELECT number FROM numbers({rows}) WHERE number % 7 = 0 ORDER BY number DESC LIMIT 20",
             tsv(last_seven - 7 * index for index in range(20))),
        Case("filtered-top-asc", f"SELECT number FROM numbers({rows}) WHERE number % 7 = 0 ORDER BY number ASC LIMIT 20",
             tsv(7 * index for index in range(20))),
        Case("full-top-desc", f"SELECT number FROM numbers({rows}) ORDER BY number DESC LIMIT 20",
             tsv(rows - 1 - index for index in range(20))),
        Case("offset-page", f"SELECT number FROM numbers({rows}) LIMIT 20 OFFSET {middle}",
             tsv(range(middle, middle + 20))),
        Case("filtered-offset", f"SELECT number FROM numbers({rows}) WHERE number % 7 = 0 LIMIT 20 OFFSET 50",
             tsv(7 * index for index in range(50, 70))),
        Case("distinct-short", f"SELECT DISTINCT number FROM numbers({rows}) WHERE number < 20 ORDER BY number DESC LIMIT 10",
             tsv(range(19, 9, -1))),
        Case("group-short", f"SELECT number, count(*) FROM numbers({rows}) WHERE number < 20 GROUP BY number ORDER BY number DESC LIMIT 10",
             tsv((number, 1) for number in range(19, 9, -1))),
        Case("series-top-desc", f"SELECT number FROM generate_series(0, {rows - 1}, 3) WHERE number % 7 = 0 ORDER BY number DESC LIMIT 20",
             tsv(list(series_multiples_of_21)[-20:][::-1])),
    ]


def select_cases(cases, only):
    if only is None:
        return list(enumerate(cases, start=1))
    numbers = set()
    try:
        for fragment in only.split(","):
            limits = fragment.split("-", 1)
            first = int(limits[0])
            last = int(limits[1]) if len(limits) > 1 else first
            if first < 1 or last < first or last > len(cases):
                raise ValueError("invalid query range")
            numbers.update(range(first, last + 1))
    except ValueError as error:
        raise ValueError("--only must contain query IDs or ranges within 1..20") from error
    if not numbers:
        raise ValueError("--only cannot be empty")
    return [(index, case) for index, case in enumerate(cases, start=1) if index in numbers]


def invoke(binary, sql, timeout):
    with tempfile.TemporaryFile() as output:
        completed = subprocess.run(
            [str(binary), "sql", sql, "--format", "tsv-no-header"],
            stdout=output, stderr=subprocess.PIPE, check=False, timeout=timeout,
        )
        output.seek(0)
        payload = output.read(256 * 1024 + 1)
    if completed.returncode:
        message = completed.stderr.decode("utf-8", "replace")[:2000]
        raise RuntimeError(f"exit {completed.returncode}: {message}")
    if len(payload) > 256 * 1024:
        raise ValueError("generated SQL benchmark output exceeds 256 KiB")
    return payload


def benchmark_one(index, case, execute, runs, warmups):
    timings = []
    status = "passed"
    error = None
    for iteration in range(-warmups, runs):
        started = time.perf_counter_ns() if iteration >= 0 else None
        try:
            actual = execute(case.sql)
        except subprocess.TimeoutExpired:
            status, error = "timeout", "SQL query exceeded the per-run timeout"
            break
        except (OSError, RuntimeError, ValueError) as exception:
            status, error = "error", str(exception)[:2000]
            break
        elapsed = (time.perf_counter_ns() - started) / 1e9 if started is not None else None
        if actual != case.expected:
            status = "mismatch"
            error = f"expected {case.expected[:120]!r}, got {actual[:120]!r}"
            break
        if elapsed is not None:
            timings.append(elapsed)
    if status != "passed":
        timings = []  # Never report partial latencies for an invalid query.
    return {
        "id": index, "name": case.name, "sql": case.sql, "status": status,
        "seconds": timings, "latency": latency_summary(timings), "error": error,
    }


def run_benchmark(cases, execute, runs, warmups):
    return [benchmark_one(index, case, execute, runs, warmups) for index, case in cases]


def optional_version(command):
    try:
        completed = subprocess.run(
            command, cwd=ROOT, capture_output=True, text=True, check=False, timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return completed.stdout.strip() if completed.returncode == 0 else None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Built release RustHouse CLI")
    parser.add_argument("--rows", type=int, default=65_536)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--only", help="Query IDs, for example 1,4,10-13")
    parser.add_argument("--output", type=Path, help="Save full JSON results")
    args = parser.parse_args(argv)
    if (args.runs < 1 or args.warmups < 0
            or not math.isfinite(args.timeout) or args.timeout <= 0):
        parser.error("--runs and --timeout must be positive; --warmups cannot be negative")
    try:
        cases = select_cases(make_cases(args.rows), args.only)
    except ValueError as error:
        parser.error(str(error))
    execute = lambda sql: invoke(args.binary, sql, args.timeout)
    results = run_benchmark(cases, execute, args.runs, args.warmups)
    report = {
        "suite": "generated-sql-latency",
        "timing_scope": "wall-clock CLI startup, query execution, and TSV output; warm OS cache",
        "rows": args.rows, "runs": args.runs, "warmups": args.warmups,
        "machine": {
            "revision": os.environ.get("GITHUB_SHA") or optional_version(["git", "rev-parse", "HEAD"]),
            "rustc": optional_version(["rustc", "--version"]),
            "platform": platform.platform(), "cpu_count": os.cpu_count(),
        },
        "results": results,
    }
    print("ID  Query                       Median ms     P90 ms    Status", file=sys.stderr)
    for result in results:
        latency = result["latency"]
        median = f'{latency["median_seconds"] * 1000:.3f}' if latency else "-"
        p90 = f'{latency["p90_seconds"] * 1000:.3f}' if latency else "-"
        print(f'{result["id"]:02}  {result["name"]:<25} {median:>9} {p90:>10}    {result["status"]}',
              file=sys.stderr)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
    emit_json(report, args.output)
    return int(any(result["status"] != "passed" for result in results))


if __name__ == "__main__":
    raise SystemExit(main())

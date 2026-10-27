#!/usr/bin/env python3
"""Bench a full-probe, two-table equi-join with verified output cardinality."""
import argparse
import json
import math
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

QUERY = "SELECT f.id, d.label FROM facts f JOIN dimensions d ON f.id = d.id"

def write_fixture(directory, probe_rows, build_rows):
    if probe_rows < 1 or not 1 <= build_rows <= 32768:
        raise ValueError("probe rows must be positive; build rows must be in 1..32768")
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    step = max(1, probe_rows // build_rows)
    expected_ids = {n * step for n in range(build_rows) if n * step < probe_rows}
    with (directory / "facts.tsv").open("w", encoding="utf-8") as facts:
        facts.write("id\tamount\n")
        for index in range(probe_rows):
            facts.write(f"{index}\t{index % 100}\n")
    with (directory / "dimensions.tsv").open("w", encoding="utf-8") as dimensions:
        dimensions.write("id\tlabel\n")
        for index in range(build_rows):
            dimensions.write(f"{index * step}\tgroup-{index % 10}\n")
    return expected_ids

def execute(binary, database, sql, timeout):
    result = subprocess.run(
        [str(binary), "sql", sql, "--database", str(database),
         "--format", "tsv-no-header"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(
            f"exit {result.returncode}: "
            + result.stderr.decode("utf-8", "replace")[:1000]
        )
    return result.stdout

def verify_result(output, expected_ids):
    lines = output.splitlines()
    if len(lines) != len(expected_ids):
        raise ValueError(
            f"expected {len(expected_ids)} join rows, received {len(lines)}"
        )
    ids = set()
    for row in lines:
        columns = row.decode("utf-8").split("\t")
        if len(columns) != 2:
            raise ValueError(f"unexpected join row: {row!r}")
        identifier = int(columns[0])
        if identifier in ids:
            raise ValueError(f"duplicate join key: {identifier}")
        ids.add(identifier)
    if ids != expected_ids:
        raise ValueError("join result IDs differ from expected matches")

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--probe-rows", type=int, default=65536)
    parser.add_argument("--build-rows", type=int, default=8192)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.runs < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error("invalid run count, warmups, or timeout")
    with tempfile.TemporaryDirectory(prefix="rusthouse-join-bench-") as root:
        root = Path(root)
        try:
            expected = write_fixture(root, args.probe_rows, args.build_rows)
        except ValueError as exc:
            parser.error(str(exc))
        database = root / "db"
        run = lambda sql: execute(args.binary, database, sql, args.timeout)
        statements = [
            "CREATE TABLE facts (id UInt64, amount UInt64)",
            "CREATE TABLE dimensions (id UInt64, label String)",
            f"COPY facts FROM '{root / 'facts.tsv'}' WITH (FORMAT TSV, HEADER TRUE)",
            f"COPY dimensions FROM '{root / 'dimensions.tsv'}' WITH (FORMAT TSV, HEADER TRUE)",
        ]
        for statement in statements:
            run(statement)
        elapsed = []
        for iteration in range(-args.warmups, args.runs):
            start = time.perf_counter() if iteration >= 0 else None
            output = run(QUERY)
            if start is not None:
                elapsed.append(time.perf_counter() - start)
            verify_result(output, expected)
    times = sorted(elapsed)
    median = statistics.median(times)
    report = {
        "engine": "rusthouse-cli",
        "timing_scope": "end-to-end CLI join including output encoding",
        "probe_rows": args.probe_rows,
        "build_rows": args.build_rows,
        "expected_matches": len(expected),
        "runs": args.runs, "warmups": args.warmups,
        "median_seconds": median,
        "p90_seconds": times[math.ceil(0.9 * len(times)) - 1],
        "logical_probe_rows_per_second": args.probe_rows / median,
        "samples_seconds": elapsed,
    }
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())

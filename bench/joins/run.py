#!/usr/bin/env python3
"""Bench a full-probe, two-table equi-join with verified output cardinality."""
import argparse
import math
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary

QUERY = "SELECT f.id, d.label FROM facts f JOIN dimensions d ON f.id = d.id"

def write_facts(path, probe_rows):
    with path.open("w", encoding="utf-8") as facts:
        facts.write("id\tamount\n")
        for index in range(probe_rows):
            facts.write(f"{index}\t{index % 100}\n")


def write_dimensions(path, build_rows, step):
    with path.open("w", encoding="utf-8") as dimensions:
        dimensions.write("id\tlabel\n")
        for index in range(build_rows):
            dimensions.write(f"{index * step}\tgroup-{index % 10}\n")


def write_fixture(directory, probe_rows, build_rows):
    if probe_rows < 1 or not 1 <= build_rows <= 32768:
        raise ValueError("probe rows must be positive; build rows must be in 1..32768")
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    step = max(1, probe_rows // build_rows)
    expected_labels = {
        n * step: f"group-{n % 10}"
        for n in range(build_rows) if n * step < probe_rows
    }
    write_facts(directory / "facts.tsv", probe_rows)
    write_dimensions(directory / "dimensions.tsv", build_rows, step)
    return expected_labels

def execute(binary, database, sql, timeout):
    return cli_sql(binary, database, sql, timeout)

def verify_result(output, expected_labels):
    if output and not output.endswith(b"\n"):
        raise ValueError("join output is missing its final newline")
    lines = output.split(b"\n")[:-1] if output else []
    if len(lines) != len(expected_labels):
        raise ValueError(
            f"expected {len(expected_labels)} join rows, received {len(lines)}"
        )
    ids = set()
    for row in lines:
        columns = row.decode("utf-8").split("\t")
        if len(columns) != 2:
            raise ValueError(f"unexpected join row: {row!r}")
        identifier_text = columns[0]
        identifier = int(identifier_text)
        if identifier_text != str(identifier):
            raise ValueError(f"noncanonical join key: {identifier_text!r}")
        if identifier in ids:
            raise ValueError(f"duplicate join key: {identifier}")
        if expected_labels.get(identifier) != columns[1]:
            raise ValueError(f"incorrect join label for key {identifier}")
        ids.add(identifier)
    if ids != expected_labels.keys():
        raise ValueError("join result IDs differ from expected matches")

def logical_probe_throughput(probe_rows, median_seconds):
    if not math.isfinite(median_seconds) or median_seconds <= 0:
        raise ValueError("median JOIN latency must be positive and finite")
    return probe_rows / median_seconds


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
    if (
        args.runs < 1
        or args.warmups < 0
        or not math.isfinite(args.timeout)
        or args.timeout <= 0
    ):
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
    latency = latency_summary(elapsed)
    median = latency["median_seconds"]
    report = {
        "engine": "rusthouse-cli",
        "timing_scope": "end-to-end CLI join including output encoding",
        "probe_rows": args.probe_rows,
        "build_rows": args.build_rows,
        "expected_matches": len(expected),
        "runs": args.runs, "warmups": args.warmups,
        "median_seconds": median,
        "p90_seconds": latency["p90_seconds"],
        "logical_probe_rows_per_second": logical_probe_throughput(args.probe_rows, median),
        "samples_seconds": elapsed,
    }
    emit_json(report, args.output)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())

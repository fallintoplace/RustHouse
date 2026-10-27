#!/usr/bin/env python3
"""Measure local table import throughput using deterministic temperature readings."""

import argparse
import json
import math
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def temperature_tenths(row):
    return (row * 17 + 13) % 801 - 400


def write_input(path, rows, stations, delimiter="\t"):
    """Create an input file and return its numeric min and max oracle."""
    minimum = 10**9
    maximum = -(10**9)
    total = 0
    with path.open("w", encoding="utf-8", newline="\n") as output:
        for row in range(rows):
            value = temperature_tenths(row)
            minimum = min(minimum, value)
            maximum = max(maximum, value)
            total += value
            output.write(
                f"station-{row % stations:05}{delimiter}{value / 10:.1f}\n"
            )
    return minimum / 10, maximum / 10, total / 10


def summarize(seconds):
    ordered = sorted(seconds)
    return {
        "median_seconds": statistics.median(ordered),
        "p90_seconds": ordered[math.ceil(len(ordered) * 0.9) - 1],
        "min_seconds": ordered[0],
        "max_seconds": ordered[-1],
    }


def sql(binary, database, statement, timeout):
    completed = subprocess.run(
        [
            str(binary),
            "sql",
            statement,
            "--database",
            str(database),
            "--format",
            "tsv-no-header",
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout,
    )
    if completed.returncode:
        raise RuntimeError(
            f"SQL failed: {statement}: {completed.stderr[:2000]}"
        )
    return completed.stdout.strip()


def verify(binary, database, rows, minimum, maximum, total, timeout):
    count = int(sql(binary, database, "SELECT count(*) FROM measurements", timeout))
    actual_min = float(
        sql(binary, database, "SELECT min(temperature) FROM measurements", timeout)
    )
    actual_max = float(
        sql(binary, database, "SELECT max(temperature) FROM measurements", timeout)
    )
    actual_sum = float(
        sql(binary, database, "SELECT sum(temperature) FROM measurements", timeout)
    )
    if (
        count != rows
        or not math.isclose(actual_min, minimum, abs_tol=1e-9)
        or not math.isclose(actual_max, maximum, abs_tol=1e-9)
        or not math.isclose(actual_sum, total, abs_tol=1e-6)
    ):
        raise RuntimeError(
            f"import result mismatch: count={count}, min={actual_min}, max={actual_max}, sum={actual_sum}"
        )


def benchmark(binary, rows, stations, runs, warmups, timeout, format_name):
    if rows < 1 or stations < 1 or stations > rows:
        raise ValueError("rows must be positive and stations must be within 1..rows")
    if runs < 1 or warmups < 0 or timeout <= 0:
        raise ValueError("runs and timeout must be positive; warmups must be nonnegative")
    if format_name not in ("TSV", "SSV"):
        raise ValueError("format must be TSV or SSV")

    with tempfile.TemporaryDirectory(prefix="rusthouse-measurements-") as directory:
        root = Path(directory)
        source = root / "measurements.txt"
        delimiter = ";" if format_name == "SSV" else "\t"
        minimum, maximum, total = write_input(source, rows, stations, delimiter)
        size = source.stat().st_size
        timings = []
        for index in range(warmups + runs):
            database = root / f"database-{index}"
            database.mkdir()
            sql(
                binary,
                database,
                "CREATE TABLE measurements (station String, temperature Float64)",
                timeout,
            )
            copy = (
                f"COPY measurements FROM '{source}' "
                f"WITH (FORMAT {format_name})"
            )
            start = time.perf_counter_ns()
            sql(binary, database, copy, timeout)
            elapsed = (time.perf_counter_ns() - start) / 1_000_000_000
            verify(binary, database, rows, minimum, maximum, total, timeout)
            if index >= warmups:
                timings.append(elapsed)
    latency = summarize(timings)
    return {
        "format": format_name,
        "rows": rows,
        "stations": stations,
        "file_bytes": size,
        "runs": runs,
        "warmups": warmups,
        "latency": latency,
        "rows_per_second": rows / latency["median_seconds"],
        "megabytes_per_second": size / latency["median_seconds"] / 1_000_000,
        "correctness": "verified",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--rows", type=int, default=100_000)
    parser.add_argument("--stations", type=int, default=413)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--format", choices=["TSV", "SSV"], default="TSV")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    result = benchmark(
        args.binary,
        args.rows,
        args.stations,
        args.runs,
        args.warmups,
        args.timeout,
        args.format,
    )
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

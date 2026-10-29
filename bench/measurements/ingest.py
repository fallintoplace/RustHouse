#!/usr/bin/env python3
"""Measure local table import throughput using deterministic temperature readings."""

import argparse
import sys
import math
import re
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json, latency_summary


def temperature_tenths(row):
    return (row * 17 + 13) % 801 - 400


def write_input(path, rows, stations, delimiter="\t", batch_rows=4096):
    """Create an input file and return its min, max, and sum oracle."""
    if type(batch_rows) is not int or not 1 <= batch_rows <= 65536:
        raise ValueError("batch_rows must be in 1..65536")
    minimum = 10**9
    maximum = -(10**9)
    total = 0
    with path.open("w", encoding="utf-8", newline="\n") as output:
        for first in range(0, rows, batch_rows):
            pending = []
            for row in range(first, min(first + batch_rows, rows)):
                value = temperature_tenths(row)
                minimum = min(minimum, value)
                maximum = max(maximum, value)
                total += value
                pending.append(f"station-{row % stations:05}{delimiter}{value / 10:.1f}\n")
            output.write("".join(pending))
    return minimum / 10, maximum / 10, total / 10


def summarize(seconds):
    return latency_summary(seconds)


def sql(binary, database, statement, timeout):
    return cli_sql(binary, database, statement, timeout).decode("utf-8").strip()


def verify(binary, database, rows, stations, minimum, maximum, total, timeout):
    count = int(sql(binary, database, "SELECT count(*) FROM measurements", timeout))
    distinct_stations = int(
        sql(binary, database, "SELECT count(DISTINCT station) FROM measurements", timeout)
    )
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
        or distinct_stations != stations
        or not math.isclose(actual_min, minimum, rel_tol=0, abs_tol=1e-9)
        or not math.isclose(actual_max, maximum, rel_tol=0, abs_tol=1e-9)
        or not math.isclose(actual_sum, total, rel_tol=0, abs_tol=1e-6)
    ):
        raise RuntimeError(
            f"import result mismatch: count={count}, stations={distinct_stations}, "
            f"min={actual_min}, max={actual_max}, sum={actual_sum}"
        )


def verified_part_count(binary, database, rows, timeout):
    # Keep the storage health check outside measured COPY timing.
    checked = subprocess.run(
        [str(binary), "doctor", "--database", str(database)],
        capture_output=True, check=False, timeout=timeout,
    )
    output = checked.stdout.decode("utf-8", "replace")
    matched = re.search(rf"^OK measurements: {rows} rows, ([0-9]+) active parts", output, re.M)
    if checked.returncode or matched is None:
        raise RuntimeError("physical part verification failed after ingestion")
    count = int(matched.group(1))
    if count < 1 or count > 16384:
        raise RuntimeError("physical part count is out of bounds")
    return count


def benchmark(binary, rows, stations, runs, warmups, timeout, format_name,
              fixture_batch_rows=4096):
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
        minimum, maximum, total = write_input(source, rows, stations, delimiter,
                                              batch_rows=fixture_batch_rows)
        size = source.stat().st_size
        timings = []
        part_counts = []
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
            verify(binary, database, rows, stations, minimum, maximum, total, timeout)
            parts = verified_part_count(binary, database, rows, timeout)
            if index >= warmups:
                timings.append(elapsed)
                part_counts.append(parts)
    latency = summarize(timings)
    return {
        "format": format_name,
        "rows": rows,
        "stations": stations,
        "fixture_batch_rows": fixture_batch_rows,
        "file_bytes": size,
        "runs": runs,
        "warmups": warmups,
        "latency": latency,
        "physical_part_counts": part_counts,
        "median_physical_parts": statistics.median(part_counts),
        "physical_parts_verified": True,
        "rows_per_second": rows / latency["median_seconds"],
        "megabytes_per_second": size / latency["median_seconds"] / 1_000_000,
        "correctness": "verified",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--rows", type=int, default=100_000)
    parser.add_argument("--stations", type=int, default=413)
    parser.add_argument("--fixture-batch-rows", type=int, default=4096)
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
        fixture_batch_rows=args.fixture_batch_rows,
    )
    emit_json(result, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

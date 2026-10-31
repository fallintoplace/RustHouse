"""Reference-check COUNT and SUM over physical UInt64 ID windows.

Usage: python -m bench.scale.physical_range_oracle --binary target/release/rusthouse
       --database ./db --table events --rows 1000000000 --window-rows 10000000
Input is expected to contain every ID in [start, start+rows) exactly once.
"""
import argparse
import json
import math
import os
import tempfile
import re
import statistics
import subprocess
import time


def expected(start, count):
    if type(start) is not int or type(count) is not int or start < 0 or count < 1:
        raise ValueError("invalid expected UInt64 ID interval")
    if start + count > 2**64:
        raise ValueError("UInt64 interval overflow")
    return count, count * (2 * start + count - 1) // 2


def sql_for(table, first, count, column="id"):
    if not isinstance(table, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", table):
        raise ValueError("unsafe table name")
    if not isinstance(column, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]{0,127}", column):
        raise ValueError("unsafe column name")
    expected(first, count)
    end = first + count
    upper = (f"{column} < {end}" if end < 2**64
             else f"{column} <= {2**64 - 1}")
    return (f"SELECT count(*), sum({column}) FROM {table} WHERE "
            f"{column} >= {first} AND {upper}")


MAX_ORACLE_RESULT_BYTES = 1024


def invoke(binary, database, statement, timeout):
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        response = subprocess.run(
            [str(binary), "sql", statement, "--database", str(database),
             "--format", "tsv-no-header"],
            stdout=output, stderr=errors, timeout=timeout, check=False,
        )
        if response.returncode:
            errors.seek(0, os.SEEK_END)
            errors.seek(max(0, errors.tell() - 500))
            raise RuntimeError(errors.read().decode("utf-8", "replace"))
        output.seek(0)
        result = output.read(MAX_ORACLE_RESULT_BYTES + 1)
    if len(result) > MAX_ORACLE_RESULT_BYTES:
        raise ValueError("physical range oracle output exceeds 1 KiB")
    return result


def verify(binary, database, table, start, rows, window_rows=10_000_000,
           timeout=1800, execute=None, column="id", max_total_seconds=None):
    expected(start, rows)
    if type(window_rows) is not int or window_rows < 1:
        raise ValueError("window_rows must be positive")
    if (rows - 1) // window_rows + 1 > 10_000:
        raise ValueError("range oracle exceeds 10000 query windows")
    if (type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("timeout must be finite and positive")
    if (max_total_seconds is not None and
            (type(max_total_seconds) not in (int, float) or
             not math.isfinite(max_total_seconds) or max_total_seconds <= 0)):
        raise ValueError("max_total_seconds must be finite and positive")
    started = time.monotonic()
    seconds = []
    count_total = 0
    sum_total = 0
    for position in range(start, start + rows, window_rows):
        remaining = None
        if max_total_seconds is not None:
            remaining = max_total_seconds - (time.monotonic() - started)
            if remaining <= 0:
                raise TimeoutError("physical range oracle total deadline exceeded")
        count = min(window_rows, start + rows - position)
        query = sql_for(table, position, count, column=column)
        begun = time.perf_counter()
        if execute is None:
            query_timeout = min(timeout, remaining) if remaining is not None else timeout
            data = invoke(binary, database, query, query_timeout)
        else:
            data = execute(query)
        if (max_total_seconds is not None and
                time.monotonic() - started > max_total_seconds):
            raise TimeoutError("physical range oracle total deadline exceeded")
        seconds.append(time.perf_counter() - begun)
        reference = expected(position, count)
        expected_wire = f"{reference[0]}\t{reference[1]}\n".encode("ascii")
        if data != expected_wire:
            raise ValueError(f"range {position}..{position + count}: expected {reference}, got {data[:100]!r}")
        count_total += reference[0]
        sum_total += reference[1]
    if (count_total, sum_total) != expected(start, rows):
        raise ValueError("verified window totals differ from the overall oracle")
    ordered = sorted(seconds)
    return {"verified": True, "rows": count_total, "sum": sum_total,
            "windows": len(seconds), "median_seconds": statistics.median(seconds),
            "p95_seconds": ordered[math.ceil(0.95 * len(ordered)) - 1],
            "p99_seconds": ordered[math.ceil(0.99 * len(ordered)) - 1],
            "max_seconds": ordered[-1]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--database", required=True)
    parser.add_argument("--table", required=True)
    parser.add_argument("--column", default="id")
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--window-rows", type=int, default=10_000_000)
    parser.add_argument("--timeout", type=float, default=1800)
    parser.add_argument("--max-total-seconds", type=float,
                        help="Total deadline for all query windows")
    args = parser.parse_args(argv)
    try:
        result = verify(args.binary, args.database, args.table, args.start, args.rows,
                        args.window_rows, args.timeout, column=args.column,
                        max_total_seconds=args.max_total_seconds)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"physical range oracle failed: {error}\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

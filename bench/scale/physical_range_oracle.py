"""Reference-check COUNT and SUM over physical UInt64 ID windows.

Usage: python -m bench.scale.physical_range_oracle --binary target/release/rusthouse
       --database ./db --table events --rows 1000000000 --window-rows 10000000
Input is expected to contain every ID in [start, start+rows) exactly once.
"""
import argparse
import json
import math
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


def sql_for(table, first, count):
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", table):
        raise ValueError("unsafe table name")
    expected(first, count)
    stop = first + count
    upper = (f"id < {stop}" if stop < 2**64
             else f"id <= {2**64 - 1}")
    return (f"SELECT count(*), sum(id) FROM {table} WHERE "
            f"id >= {first} AND {upper}")


def invoke(binary, database, statement, timeout):
    response = subprocess.run(
        [str(binary), "sql", statement, "--database", str(database),
         "--format", "tsv-no-header"],
        capture_output=True, timeout=timeout, check=False,
    )
    if response.returncode:
        raise RuntimeError(response.stderr.decode("utf-8", "replace")[-500:])
    return response.stdout


def verify(binary, database, table, start, rows, window_rows=10_000_000,
           timeout=1800, execute=None):
    expected(start, rows)
    if type(window_rows) is not int or window_rows < 1:
        raise ValueError("window_rows must be positive")
    if (rows - 1) // window_rows + 1 > 10_000:
        raise ValueError("range oracle exceeds 10000 query windows")
    if timeout <= 0 or not math.isfinite(timeout):
        raise ValueError("timeout must be finite and positive")
    execute = execute or (lambda statement: invoke(binary, database, statement, timeout))
    seconds = []
    count_total = 0
    sum_total = 0
    stop = start + rows
    for position in range(start, stop, window_rows):
        count = min(window_rows, stop - position)
        query = sql_for(table, position, count)
        begun = time.perf_counter()
        data = execute(query)
        seconds.append(time.perf_counter() - begun)
        reference = expected(position, count)
        expected_wire = f"{reference[0]}\t{reference[1]}\n".encode("ascii")
        if data != expected_wire:
            raise ValueError(f"range {position}..{position + count}: expected {reference}, got {data[:100]!r}")
        count_total += reference[0]
        sum_total += reference[1]
    if (count_total, sum_total) != expected(start, rows):
        raise ValueError("verified window totals differ from the overall oracle")
    return {"verified": True, "rows": count_total, "sum": sum_total,
            "windows": len(seconds), "median_seconds": statistics.median(seconds),
            "max_seconds": max(seconds)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--database", required=True)
    parser.add_argument("--table", required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--window-rows", type=int, default=10_000_000)
    parser.add_argument("--timeout", type=float, default=1800)
    args = parser.parse_args(argv)
    try:
        result = verify(args.binary, args.database, args.table, args.start, args.rows,
                        args.window_rows, args.timeout)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"physical range oracle failed: {error}\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

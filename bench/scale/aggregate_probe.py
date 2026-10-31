"""Verify analytical aggregation on a physical sequential-UInt64 snapshot.

Queries operate on stored parts, not generated numbers(). Assumes the chosen
column contains each integer in [0, rows) exactly once.
"""
import argparse
import json
import math
import re
import subprocess
import tempfile
import time
from pathlib import Path


def aggregates(rows, *, start=0, column="id"):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    if (type(start) is not int or start < 0 or start + rows > (1 << 64)):
        raise ValueError("invalid UInt64 interval")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", column):
        raise ValueError("column must be a SQL identifier")
    last_id = start + rows - 1
    total = rows * (start + last_id) // 2
    return (("count(*)", rows), (f"sum({column})", total),
            (f"min({column})", start), (f"max({column})", last_id))


def verify(binary, part, rows, *, timeout=600, execute=None,
           start=0, column="id"):
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    result = []
    for expression, expected in aggregates(rows, start=start, column=column):
        sql = f"SELECT {expression} FROM part"
        command = [str(binary), "sql", sql, "--part", str(part),
                   "--format", "tsv-no-header"]
        expected_bytes = f"{expected}\n".encode()
        started = time.perf_counter()
        if execute is None:
            with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
                completed = subprocess.run(command, stdout=output, stderr=errors,
                                           check=False, timeout=timeout)
                if completed.returncode:
                    errors.seek(0, 2)
                    errors.seek(max(0, errors.tell() - 500))
                    diagnostic = errors.read(500)
                    raise RuntimeError(f"{expression}: query failed: {diagnostic!r}")
                output.seek(0)
                actual = output.read(len(expected_bytes) + 1)
        else:
            completed = execute(command, capture_output=True,
                                check=False, timeout=timeout)
            if completed.returncode:
                raise RuntimeError(f"{expression}: query failed: {completed.stderr[:500]!r}")
            actual = completed.stdout
        elapsed = time.perf_counter() - started
        if actual != expected_bytes:
            raise ValueError(f"{expression}: result differs from independent reference")
        result.append({"aggregate": expression, "expected": expected,
                       "seconds": elapsed, "verified": True})
    return {"workload": "persisted-aggregate-probe", "rows_assumed": rows,
            "assumption": f"one record per {column} in [{start}, {start + rows})",
            "start": start, "column": column, "results": result}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--column", default="id")
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.binary, args.part, args.rows, timeout=args.timeout,
                        start=args.start, column=args.column)
        content = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(content, encoding="utf-8")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"aggregate probe failed: {error}\n")
    print(content, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

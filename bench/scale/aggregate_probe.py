"""Verify analytical aggregation on a physical sequential-UInt64 snapshot.

Queries operate on stored parts, not generated numbers(). Assumes the chosen
column contains each integer in [0, rows) exactly once.
"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path


def aggregates(rows):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    return (("count(*)", rows), ("sum(id)", rows * (rows - 1) // 2),
            ("min(id)", 0), ("max(id)", rows - 1))


def verify(binary, part, rows, *, timeout=600, execute=None):
    if timeout <= 0:
        raise ValueError("timeout must be positive")
    run = execute or subprocess.run
    result = []
    for expression, expected in aggregates(rows):
        sql = f"SELECT {expression} FROM part"
        started = time.perf_counter()
        completed = run([str(binary), "sql", sql, "--part", str(part),
                         "--format", "tsv-no-header"],
                        capture_output=True, check=False, timeout=timeout)
        elapsed = time.perf_counter() - started
        if completed.returncode:
            raise RuntimeError(f"{expression}: query failed: {completed.stderr!r}")
        if completed.stdout != f"{expected}\n".encode():
            raise ValueError(f"{expression}: result differs from independent reference")
        result.append({"aggregate": expression, "expected": expected,
                       "seconds": elapsed, "verified": True})
    return {"workload": "persisted-aggregate-probe", "rows_assumed": rows,
            "assumption": "one record per id 0..rows-1", "results": result}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.binary, args.part, args.rows, timeout=args.timeout)
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

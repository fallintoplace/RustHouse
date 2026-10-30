"""Verify ORDER BY top-K on a persisted sequential UInt64 fixture.

Requires ids 0..rows-1 exactly once, and an offline immutable part snapshot.
"""
import argparse
import json
import math
import re
import subprocess
import tempfile
import time
from pathlib import Path


def probe(binary, part, rows, *, column="id", limit=10, timeout=600,
          execute=None, start=0):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive")
    if (type(start) is not int or start < 0
            or start + rows > (1 << 64)):
        raise ValueError("invalid UInt64 start or range overflow")
    if type(limit) is not int or not 1 <= limit <= min(4096, rows):
        raise ValueError("limit must be in 1..min(4096, rows)")
    if not isinstance(column, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,127}", column):
        raise ValueError("column must be a simple SQL identifier")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    run = execute or subprocess.run
    results = []
    for descending in (False, True):
        direction = "DESC" if descending else "ASC"
        sql = f"SELECT {column} FROM part ORDER BY {column} {direction} LIMIT {limit}"
        expected_values = (range(start + rows - 1, start + rows - 1 - limit, -1)
                           if descending else range(start, start + limit))
        expected = "".join(f"{value}\n" for value in expected_values).encode()
        begun = time.perf_counter()
        command = [str(binary), "sql", sql, "--part", str(part),
                   "--format", "tsv-no-header"]
        if execute is None:
            with tempfile.TemporaryFile(mode="w+b") as sink, tempfile.TemporaryFile(mode="w+b") as errors:
                completed = run(command, stdout=sink, stderr=errors,
                                check=False, timeout=timeout)
                sink.seek(0)
                output = sink.read(len(expected) + 1)
                errors.seek(0, 2)
                errors.seek(max(0, errors.tell() - 600))
                diagnostic = errors.read(600).decode("utf-8", "replace")
        else:
            completed = run(command, capture_output=True, check=False, timeout=timeout)
            output = completed.stdout
            diagnostic = completed.stderr[-600:].decode("utf-8", "replace")
        elapsed = time.perf_counter() - begun
        if completed.returncode:
            raise RuntimeError(f"{direction} top-K failed: {diagnostic}")
        if output != expected:
            raise ValueError(f"{direction} top-K differs from exact sequential-id reference")
        results.append({"direction": direction, "limit": limit,
                        "seconds": elapsed, "verified": True})
    return {"workload": "physical-topk-probe", "rows_assumed": rows,
            "assumption": f"unique sequential ids {start}..{start + rows - 1}",
            "start": start, "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--column", default="id")
    parser.add_argument("--limit", type=int, default=10)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = probe(args.binary, args.part, args.rows, column=args.column,
                       limit=args.limit, timeout=args.timeout, start=args.start)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"top-K probe failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

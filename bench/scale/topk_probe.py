"""Verify ORDER BY top-K on a persisted sequential UInt64 fixture.

Requires ids 0..rows-1 exactly once, and an offline immutable part snapshot.
"""
import argparse
import json
import math
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path


def probe(binary, part, rows, *, column="id", limit=None, timeout=600,
          execute=None, start=0):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive")
    if (type(start) is not int or start < 0
            or start + rows > (1 << 64)):
        raise ValueError("invalid UInt64 start or range overflow")
    # Report invalid security-sensitive inputs before the default limit is
    # checked. Small fixtures also need an automatic limit below ten rows
    # for an unsafe identifier or an invalid subprocess timeout.
    if not isinstance(column, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,127}", column):
        raise ValueError("column must be a simple SQL identifier")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if limit is None:
        limit = min(10, rows)
    if type(limit) is not int or not 1 <= limit <= min(4096, rows):
        raise ValueError("limit must be in 1..min(4096, rows)")
    last_id = start + rows - 1
    run = execute or subprocess.run
    results = []
    for descending in (False, True):
        direction = "DESC" if descending else "ASC"
        sql = f"SELECT {column} FROM part ORDER BY {column} {direction} LIMIT {limit}"
        expected_values = (range(last_id, last_id - limit, -1)
                           if descending else range(start, start + limit))
        expected = "".join(f"{value}\n" for value in expected_values).encode()
        begun = time.perf_counter()
        command = [str(binary), "sql", sql, "--part", str(part),
                   "--format", "tsv-no-header"]
        if execute is None:
            extra = {}
            if os.name == "posix":
                import resource

                def cap_topk_output():
                    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                    limit = len(expected) + 1
                    if soft != resource.RLIM_INFINITY:
                        limit = min(limit, soft)
                    if hard != resource.RLIM_INFINITY:
                        limit = min(limit, hard)
                    resource.setrlimit(resource.RLIMIT_FSIZE, (limit, hard))

                extra["preexec_fn"] = cap_topk_output
            with tempfile.TemporaryFile(mode="w+b") as sink, tempfile.TemporaryFile(mode="w+b") as errors:
                completed = run(command, stdout=sink, stderr=errors,
                                check=False, timeout=timeout, **extra)
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
            "assumption": f"unique sequential ids {start}..{last_id}",
            "start": start, "results": results}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--column", default="id")
    parser.add_argument("--limit", type=int)
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

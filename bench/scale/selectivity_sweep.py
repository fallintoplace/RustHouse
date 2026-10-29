"""Measure physically stored sorted-UInt64 predicate selectivity with exact references.

Requires the frozen snapshot to contain each id in 0..rows-1 exactly once.
Timing includes a new CLI process and potentially warm OS file caches.
"""
import argparse
import json
import math
import statistics
import subprocess
import time
from pathlib import Path


def thresholds(rows):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    return sorted(set((0, rows // 4, rows // 2, rows * 3 // 4, rows)))


def run_sweep(binary, part, rows, *, samples=3, timeout=600, execute=None):
    if (type(samples) is not int or not 1 <= samples <= 128
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("samples must be positive integers and timeout finite and positive")
    run = execute or subprocess.run
    results = []
    for threshold in thresholds(rows):
        sql = f"SELECT count(*) FROM part WHERE id >= {threshold}"
        expected = f"{rows - threshold}\n".encode()
        durations = []
        for _ in range(samples):
            start = time.perf_counter()
            completed = run(
                [str(binary), "sql", sql, "--part", str(part), "--format", "tsv-no-header"],
                capture_output=True, check=False, timeout=timeout,
            )
            elapsed = time.perf_counter() - start
            if completed.returncode:
                raise RuntimeError(f"failed predicate threshold {threshold}: {completed.stderr!r}")
            if completed.stdout != expected:
                raise ValueError(f"threshold {threshold}: result differs from sequential-id oracle")
            durations.append(elapsed)
        results.append({"threshold": threshold, "expected_count": rows - threshold,
                        "median_seconds": statistics.median(durations),
                        "samples_seconds": durations, "verified": True})
    return {"workload": "physical-selectivity-sweep", "rows_assumed": rows,
            "assumption": "id is exactly 0..rows-1", "results": results,
            "measures_physical_scan": "depends on optimizer; consult EXPLAIN"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_sweep(args.binary, args.part, args.rows, samples=args.samples,
                           timeout=args.timeout)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"selectivity sweep failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

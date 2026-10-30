"""Measure physically stored sorted-UInt64 predicate selectivity with exact references.

Requires the frozen snapshot to contain each id in 0..rows-1 exactly once.
Timing includes a new CLI process and potentially warm OS file caches.
"""
import argparse
import json
import math
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def thresholds(rows, steps=4):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    if type(steps) is not int or not 1 <= steps <= 64:
        raise ValueError("steps must be in 1..64")
    return sorted({rows * numerator // steps for numerator in range(steps + 1)})


MAX_RESULT_BYTES = 64


def execute_predicate(command, timeout, runner=None):
    if runner is not None:
        result = runner(command, capture_output=True, check=False, timeout=timeout)
        raw = result.stdout
        diagnostic = result.stderr[-500:]
        returncode = result.returncode
    else:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            result = subprocess.run(command, stdout=output, stderr=errors,
                                    check=False, timeout=timeout)
            output.seek(0)
            raw = output.read(MAX_RESULT_BYTES + 1)
            errors.seek(max(0, errors.tell() - 500))
            diagnostic = errors.read(500)
            returncode = result.returncode
    if len(raw) > MAX_RESULT_BYTES:
        raise ValueError("predicate sweep result exceeds 64 bytes")
    return returncode, raw, diagnostic


def run_sweep(binary, part, rows, *, samples=3, timeout=600,
              execute=None, steps=4):
    if (type(samples) is not int or not 1 <= samples <= 128
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("samples must be positive integers and timeout finite and positive")
    selected_thresholds = thresholds(rows, steps)
    if len(selected_thresholds) * samples > 1_024:
        raise ValueError("selectivity sweep exceeds 1024 SQL executions")
    results = []
    for threshold in selected_thresholds:
        sql = f"SELECT count(*) FROM part WHERE id >= {threshold}"
        expected = f"{rows - threshold}\n".encode()
        durations = []
        for _ in range(samples):
            start = time.perf_counter()
            returncode, observed, diagnostic = execute_predicate(
                [str(binary), "sql", sql, "--part", str(part), "--format", "tsv-no-header"],
                timeout, execute,
            )
            elapsed = time.perf_counter() - start
            if returncode:
                raise RuntimeError(f"failed predicate threshold {threshold}: {diagnostic!r}")
            if observed != expected:
                raise ValueError(f"threshold {threshold}: result differs from sequential-id oracle")
            durations.append(elapsed)
        results.append({"threshold": threshold, "expected_count": rows - threshold,
                        "selectivity_percent": round(100 * (rows - threshold) / rows, 6),
                        "median_seconds": statistics.median(durations),
                        "p95_seconds": sorted(durations)[math.ceil(0.95 * len(durations)) - 1],
                        "samples_seconds": durations, "verified": True})
    return {"workload": "physical-selectivity-sweep", "rows_assumed": rows,
            "assumption": "id is exactly 0..rows-1", "steps": steps,
            "results": results,
            "measures_physical_scan": "depends on optimizer; consult EXPLAIN"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--steps", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_sweep(args.binary, args.part, args.rows, samples=args.samples,
                           timeout=args.timeout, steps=args.steps)
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

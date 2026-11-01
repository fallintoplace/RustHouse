"""Measure physically stored sorted-UInt64 predicate selectivity with exact references.

Requires a frozen snapshot containing each ID in the configured contiguous range once.
Timing includes a new CLI process and potentially warm OS file caches.
"""
import argparse
import json
import math
import os
import subprocess
import tempfile
import time
from pathlib import Path


def thresholds(rows, steps=4, *, start=0):
    if type(rows) is not int or not 1 <= rows <= (1 << 64) - 1:
        raise ValueError("rows must be positive UInt64")
    if type(steps) is not int or not 1 <= steps <= 64:
        raise ValueError("steps must be in 1..64")
    if type(start) is not int or start < 0 or start + rows > (1 << 64) - 1:
        raise ValueError("invalid UInt64 start or unrepresentable upper threshold")
    return sorted({start + rows * numerator // steps for numerator in range(steps + 1)})


MAX_RESULT_BYTES = 64


def execute_predicate(command, timeout, runner=None):
    if runner is not None:
        result = runner(command, capture_output=True, check=False, timeout=timeout)
        raw = result.stdout
        diagnostic = result.stderr[-500:]
        returncode = result.returncode
    else:
        # Bound the child while it runs. Reading 65 bytes after it exits does
        # not stop a faulty query from filling the temporary filesystem.
        extra = {}
        if os.name == "posix":
            import resource

            def cap_predicate_output():
                soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
                limit = MAX_RESULT_BYTES + 1
                if soft != resource.RLIM_INFINITY:
                    limit = min(limit, soft)
                if hard != resource.RLIM_INFINITY:
                    limit = min(limit, hard)
                resource.setrlimit(resource.RLIMIT_FSIZE, (limit, hard))

            extra["preexec_fn"] = cap_predicate_output
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            result = subprocess.run(command, stdout=output, stderr=errors,
                                    check=False, timeout=timeout, **extra)
            output.seek(0)
            raw = output.read(MAX_RESULT_BYTES + 1)
            errors.seek(max(0, errors.tell() - 500))
            diagnostic = errors.read(500)
            returncode = result.returncode
    if len(raw) > MAX_RESULT_BYTES:
        raise ValueError("predicate sweep result exceeds 64 bytes")
    return returncode, raw, diagnostic


def run_sweep(binary, part, rows, *, samples=3, timeout=600,
              execute=None, steps=4, start=0):
    if (type(samples) is not int or not 1 <= samples <= 128
            or type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("samples must be positive integers and timeout finite and positive")
    selected_thresholds = thresholds(rows, steps, start=start)
    if len(selected_thresholds) * samples > 1_024:
        raise ValueError("selectivity sweep exceeds 1024 SQL executions")
    results = []
    for threshold in selected_thresholds:
        sql = f"SELECT count(*) FROM part WHERE id >= {threshold}"
        expected = f"{rows - (threshold - start)}\n".encode()
        durations = []
        for _ in range(samples):
            begun = time.perf_counter()
            returncode, observed, diagnostic = execute_predicate(
                [str(binary), "sql", sql, "--part", str(part), "--format", "tsv-no-header"],
                timeout, execute,
            )
            elapsed = time.perf_counter() - begun
            if returncode:
                raise RuntimeError(f"failed predicate threshold {threshold}: {diagnostic!r}")
            if observed != expected:
                raise ValueError(f"threshold {threshold}: result differs from sequential-id oracle")
            durations.append(elapsed)
        ordered = sorted(durations)
        middle = len(ordered) // 2
        median = (ordered[middle] if len(ordered) % 2
                  else (ordered[middle - 1] + ordered[middle]) / 2)
        results.append({"threshold": threshold, "expected_count": rows - (threshold - start),
                        "selectivity_percent": round(100 * (rows - (threshold - start)) / rows, 6),
                        "median_seconds": median,
                        "p95_seconds": ordered[math.ceil(0.95 * len(ordered)) - 1],
                        "samples_seconds": durations, "verified": True})
    return {"workload": "physical-selectivity-sweep", "rows_assumed": rows,
            "assumption": f"id is exactly {start}..{start + rows - 1}", "steps": steps,
            "start": start,
            "results": results,
            "measures_physical_scan": "depends on optimizer; consult EXPLAIN"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--steps", type=int, default=4)
    parser.add_argument("--start", type=int, default=0)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_sweep(args.binary, args.part, args.rows, samples=args.samples,
                           timeout=args.timeout, steps=args.steps, start=args.start)
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

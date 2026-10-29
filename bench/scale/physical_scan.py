"""Reference-checked analytical scans over data actually persisted in wide parts.

The fixture is generated and loaded outside timed SQL samples. COUNT and SUM
use a modulo predicate so the result cannot be inferred from table row-count
metadata alone. The temporary database is always removed after the run.
"""
import argparse
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


COUNT_SQL = "SELECT count(*) FROM events WHERE id % 2 = 0"
SUM_SQL = "SELECT sum(id) FROM events WHERE id % 2 = 0"


def even_reference(rows):
    """For input integers 0..rows-1, return even-row count and sum."""
    if type(rows) is not int or rows < 1:
        raise ValueError("rows must be positive")
    count = (rows + 1) // 2
    return count, count * (count - 1)


def p90(samples):
    if not samples:
        raise ValueError("at least one sample is required")
    return sorted(samples)[math.ceil(0.9 * len(samples)) - 1]


def write_fixture(path, rows, *, batch_rows=8192):
    if type(batch_rows) is not int or not 1 <= batch_rows <= 1_000_000:
        raise ValueError("fixture batch_rows must be in 1..1000000")
    with path.open("w", encoding="ascii", buffering=1024 * 1024) as output:
        for start in range(0, rows, batch_rows):
            output.write("".join(f"{value}\n" for value in range(start, min(start + batch_rows, rows))))


def invoke(binary, database, sql, *, timeout, no_header=False):
    command = [str(binary), "sql", sql, "--database", str(database)]
    if no_header:
        command.extend(["--format", "tsv-no-header"])
    start = time.perf_counter()
    process = subprocess.run(
        command,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    elapsed = time.perf_counter() - start
    if process.returncode:
        raise RuntimeError(
            f"{sql}: exit {process.returncode}: {process.stderr.decode('utf-8', 'replace')[-600:]}"
        )
    return process.stdout, elapsed


def scan(binary, database, sql, expected, runs, warmups, timeout):
    samples = []
    for index in range(runs + warmups):
        actual, elapsed = invoke(binary, database, sql, timeout=timeout, no_header=True)
        if actual != f"{expected}\n".encode("ascii"):
            raise ValueError(f"{sql}: expected {expected}, received {actual!r}")
        if index >= warmups:
            samples.append(elapsed)
    return {
        "verified": True,
        "median_seconds": statistics.median(samples),
        "p90_seconds": p90(samples),
        "samples_seconds": samples,
    }


def benchmark(binary, rows, runs, warmups, timeout, scratch_dir=None, fixture_batch_rows=8192):
    if type(rows) is not int or not 1 <= rows <= 1_000_000_000:
        raise ValueError("rows must be in 1..1000000000")
    if type(runs) is not int or type(warmups) is not int or runs < 1 or warmups < 0:
        raise ValueError("invalid physical scan run counts")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    count, total = even_reference(rows)
    with tempfile.TemporaryDirectory(prefix="rusthouse-physical-scan-", dir=scratch_dir) as root:
        directory = Path(root) / "db"
        tsv = Path(root) / "input.tsv"
        write_fixture(tsv, rows, batch_rows=fixture_batch_rows)
        invoke(binary, directory, "CREATE TABLE events (id UInt64)", timeout=timeout)
        escaped_path = str(tsv).replace("'", "''")
        _, ingestion_seconds = invoke(
            binary,
            directory,
            f"COPY events FROM '{escaped_path}' WITH (FORMAT TSV)",
            timeout=timeout,
        )

        inspection = subprocess.run(
            [str(binary), "doctor", "--database", str(directory)],
            capture_output=True,
            check=False,
            timeout=timeout,
        )
        expected_summary = f"OK events: {rows} rows, "
        if inspection.returncode or expected_summary not in inspection.stdout.decode("utf-8", "replace"):
            raise ValueError(
                f"physical snapshot verification failed: {inspection.stdout!r} {inspection.stderr!r}"
            )
        count_report = scan(binary, directory, COUNT_SQL, count, runs, warmups, timeout)
        sum_report = scan(binary, directory, SUM_SQL, total, runs, warmups, timeout)
        return {
            "workload": "rusthouse-persisted-wide-part-full-scans",
            "revision": os.environ.get("GITHUB_SHA"),
            "rows_written_and_verified": rows,
            "reference_checked": True,
            "source": "generated TSV loaded with COPY into a new immutable-part table",
            "note": "timings include fresh CLI processes and warm filesystem cache; not official TPC-H",
            "platform": platform.platform(),
            "cpu_count": os.cpu_count(),
            "ingestion_seconds": ingestion_seconds,
            "runs": runs,
            "warmups": warmups,
            "count_even": {"query": COUNT_SQL, "expected": count, **count_report},
            "sum_even": {"query": SUM_SQL, "expected": total, **sum_report},
        }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=100_000)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--scratch-dir", type=Path)
    parser.add_argument("--fixture-batch-rows", type=int, default=8192)
    parser.add_argument("--require-billion-rows", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)

    if not 1 <= args.rows <= 1_000_000_000:
        parser.error("--rows must be in 1..1000000000")
    if args.require_billion_rows and args.rows < 1_000_000_000:
        parser.error("--require-billion-rows needs --rows 1000000000")
    if args.runs < 1 or args.warmups < 0:
        parser.error("--runs must be positive and --warmups nonnegative")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    if args.scratch_dir is not None and not args.scratch_dir.is_dir():
        parser.error("--scratch-dir must exist")

    try:
        report = benchmark(
            args.binary, args.rows, args.runs, args.warmups, args.timeout,
            args.scratch_dir, args.fixture_batch_rows,
        )
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"persisted scan benchmark failed: {error}\n")

    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""Verify real multi-part GROUP BY COUNT/SUM on highly skewed fact tables.

Write and COPY an actual persisted data set before timing queries. Every
timed SQL result is checked against an independent O(groups * 97) arithmetic
oracle, including when requesting 1,000,000,000 rows. No generated numbers()
virtual table and no full-query metadata shortcut are used.
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
from pathlib import Path

from .physical_scan import invoke, p90


COUNT_SQL = "SELECT region, count(*) FROM events GROUP BY region ORDER BY region"
SUM_SQL = "SELECT region, sum(amount) FROM events GROUP BY region ORDER BY region"


def region_at(index, groups):
    if index % 10 != 9:
        return 0
    return 1 + (index // 10) % (groups - 1)


def emit(stream, rows, groups, batch_rows=8192):
    if not 1 <= rows <= 1_000_000_000 or not 2 <= groups <= 65_536:
        raise ValueError("unsupported row count or UInt16 group domain")
    for start in range(0, rows, batch_rows):
        stream.write("".join(
            f"{region_at(index, groups)}\t{1 + index % 97}\n"
            for index in range(start, min(rows, start + batch_rows))
        ))


def reference(rows, groups):
    """Independent arithmetic oracle for a 90% hot key plus cold cyclic keys.

    Every ten rows have nine hot-key records and one cold-key record.
    Cold bucket b appears at block b-1 + k*(groups-1). Its amount is
    1 + (10*block + 9) % 97. Evaluate only one residue cycle per bucket.
    """
    if not 1 <= rows <= 1_000_000_000 or not 2 <= groups <= 65_536:
        raise ValueError("unsupported row count or UInt16 group domain")
    counts = [0] * groups
    sums = [0] * groups
    blocks = rows // 10
    cold_sum = 0
    delta = 10 * (groups - 1)
    period = 97 // math.gcd(delta, 97)
    for bucket in range(1, groups):
        first_block = bucket - 1
        if first_block >= blocks:
            continue
        count = 1 + (blocks - 1 - first_block) // (groups - 1)
        base = 10 * first_block + 9
        full, extra = divmod(count, period)
        one_period = sum(1 + ((base + delta * k) % 97) for k in range(period))
        remainder = sum(1 + ((base + delta * k) % 97) for k in range(extra))
        counts[bucket] = count
        sums[bucket] = full * one_period + remainder
        cold_sum += sums[bucket]
    counts[0] = rows - blocks
    cycles, remainder = divmod(rows, 97)
    total_sum = cycles * (97 * 98 // 2) + remainder * (remainder + 1) // 2
    sums[0] = total_sum - cold_sum
    assert sum(counts) == rows
    assert sum(sums) == total_sum
    return counts, sums


def expected_tsv(metrics):
    return "".join(f"{region}\t{value}\n" for region, value in enumerate(metrics) if value).encode("ascii")


def scan(binary, directory, sql, golden, *, runs, warmups, timeout):
    samples = []
    for attempt in range(runs + warmups):
        output, seconds = invoke(binary, directory, sql, timeout=timeout, no_header=True)
        if output != golden:
            raise ValueError(
                f"{sql}: grouped result mismatch ({len(output)} bytes vs {len(golden)} expected)"
                f", first 250 bytes: {output[:250]!r}"
            )
        if attempt >= warmups:
            samples.append(seconds)
    return {
        "verified": True,
        "median_seconds": statistics.median(samples),
        "p90_seconds": p90(samples),
        "samples_seconds": samples,
    }


def benchmark(binary, rows, groups, runs, warmups, timeout, scratch_dir=None):
    counts, amounts = reference(rows, groups)
    with tempfile.TemporaryDirectory(prefix="rusthouse-grouped-analytics-", dir=scratch_dir) as root:
        directory = Path(root) / "db"
        fixture = Path(root) / "facts.tsv"
        with fixture.open("w", encoding="ascii", buffering=1024 * 1024) as stream:
            emit(stream, rows, groups)
        invoke(binary, directory, "CREATE TABLE events (region UInt16, amount UInt64)", timeout=timeout)
        escaped = str(fixture).replace("'", "''")
        _, ingestion_seconds = invoke(
            binary, directory, f"COPY events FROM '{escaped}' WITH (FORMAT TSV)",
            timeout=timeout,
        )
        inspection = subprocess.run(
            [str(binary), "doctor", "--database", str(directory)],
            capture_output=True, check=False, timeout=timeout,
        )
        if inspection.returncode or f"OK events: {rows} rows, " not in inspection.stdout.decode("utf-8", "replace"):
            raise RuntimeError(
                f"persisted snapshot verification failed: {inspection.stdout!r} {inspection.stderr!r}"
            )
        return {
            "workload": "persisted-billion-row-skewed-grouped-analytics",
            "revision": os.environ.get("GITHUB_SHA"),
            "rows_written_and_verified": rows,
            "groups_configured": groups,
            "groups_with_rows": sum(bool(count) for count in counts),
            "hot_key_frequency": counts[0] / rows,
            "oracle": "independent modular closed-form COUNT and SUM",
            "physical_parts_verified": True,
            "platform": platform.platform(),
            "cpu_count": os.cpu_count(),
            "ingestion_seconds": ingestion_seconds,
            "runs": runs,
            "warmups": warmups,
            "note": "Fresh CLI query processes; filesystem cache may be warm. Not a TPC-H comparison.",
            "grouped_count": {
                "sql": COUNT_SQL,
                **scan(binary, directory, COUNT_SQL, expected_tsv(counts),
                       runs=runs, warmups=warmups, timeout=timeout),
            },
            "grouped_sum": {
                "sql": SUM_SQL,
                **scan(binary, directory, SUM_SQL, expected_tsv(amounts),
                       runs=runs, warmups=warmups, timeout=timeout),
            },
        }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rows", type=int, default=1_000_000)
    parser.add_argument("--groups", type=int, default=4096)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--scratch-dir", type=Path)
    parser.add_argument("--require-billion-rows", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if not 1 <= args.rows <= 1_000_000_000:
        parser.error("--rows must be in 1..1000000000")
    if not 2 <= args.groups <= 65_536:
        parser.error("--groups must fit a UInt16 domain (2..65536)")
    if args.require_billion_rows and args.rows != 1_000_000_000:
        parser.error("--require-billion-rows needs exactly 1000000000 rows")
    if args.runs < 1 or args.warmups < 0:
        parser.error("--runs must be positive and --warmups nonnegative")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    if args.scratch_dir is not None and not args.scratch_dir.is_dir():
        parser.error("--scratch-dir must exist")
    try:
        result = benchmark(args.binary, args.rows, args.groups, args.runs,
                           args.warmups, args.timeout, args.scratch_dir)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"physical GROUP BY gate failed: {error}\n")
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())

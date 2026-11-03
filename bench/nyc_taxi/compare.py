#!/usr/bin/env python3
"""Run a pinned NYC taxi SQL coverage and DuckDB parity check.

Assumes trips and zones have been loaded into both databases from the same
source. Does not load data or infer benchmark equivalence.
"""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

WORKLOAD = Path(__file__).with_name("queries.json")


def parse_tsv(data: str) -> list[tuple[str | None, ...]]:
    lines = data.splitlines()
    if not lines:
        raise ValueError("RustHouse returned no TSV header")
    width = len(lines[0].split("\t"))
    rows: list[tuple[str | None, ...]] = []
    for line in lines[1:]:
        values = tuple(None if cell == r"\N" else cell for cell in line.split("\t"))
        if len(values) != width:
            raise ValueError(f"unexpected TSV field count: {len(values)} != {width}")
        rows.append(values)
    return rows


def value_matches(actual: str | None, expected: Any) -> bool:
    if actual is None:
        return expected is None
    if expected is None:
        return False
    if isinstance(expected, (int, float, Decimal)) and not isinstance(expected, bool):
        try:
            a = Decimal(actual)
            b = Decimal(str(expected))
        except InvalidOperation:
            return False
        if not a.is_finite() or not b.is_finite():
            return a == b
        tolerance = max(abs(b) * Decimal("0.000001"), Decimal("0.000001"))
        return abs(a - b) <= tolerance
    return actual == str(expected)


def rows_match(actual: list[tuple[str | None, ...]], expected: list[tuple[Any, ...]]) -> tuple[bool, str]:
    if len(actual) != len(expected):
        return False, f"row count: RustHouse {len(actual)}, DuckDB {len(expected)}"
    for position, (rusthouse_row, duckdb_row) in enumerate(zip(actual, expected, strict=True)):
        if len(rusthouse_row) != len(duckdb_row):
            return False, f"row {position}: column count differs"
        for index, (left, right) in enumerate(zip(rusthouse_row, duckdb_row, strict=True)):
            if not value_matches(left, right):
                return False, f"row {position}, column {index}: RustHouse {left!r}, DuckDB {right!r}"
    return True, ""


def run_rusthouse(binary: str, database: str, sql: str) -> tuple[subprocess.CompletedProcess[str], float]:
    start = time.perf_counter()
    result = subprocess.run(
        [binary, "sql", sql, "--database", database],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    return result, (time.perf_counter() - start) * 1000


def run_suite(binary: str, database: str, duckdb_path: str, repeats: int) -> int:
    try:
        import duckdb
    except ImportError as error:
        raise RuntimeError("Install DuckDB with: python -m pip install duckdb") from error
    workload = json.loads(WORKLOAD.read_text(encoding="utf-8"))
    conn = duckdb.connect(duckdb_path, read_only=True)
    passing = 0
    mismatches = 0
    errors = 0
    try:
        for query in workload:
            sql = query["sql"]
            label = f'{query["id"]:02d} {query["name"]}'
            expected = conn.execute(sql).fetchall()
            # Warm the CLI and filesystem before collecting repeat medians.
            first, _ = run_rusthouse(binary, database, sql)
            if first.returncode:
                errors += 1
                summary = first.stderr.strip().splitlines()
                print(f"{label}: SQL_ERROR: {summary[-1] if summary else first.returncode}")
                continue
            durations = []
            last = first
            for _ in range(repeats):
                last, milliseconds = run_rusthouse(binary, database, sql)
                if last.returncode:
                    break
                durations.append(milliseconds)
            if last.returncode:
                errors += 1
                print(f"{label}: SQL_ERROR during measured run: {last.stderr.strip()}")
                continue
            try:
                actual = parse_tsv(last.stdout)
                equal, reason = rows_match(actual, expected)
            except ValueError as error:
                equal, reason = False, str(error)
            median = statistics.median(durations)
            if equal:
                passing += 1
                print(f"{label}: PASS {median:.1f} ms ({len(actual)} rows)")
            else:
                mismatches += 1
                print(f"{label}: MISMATCH {median:.1f} ms: {reason}")
    finally:
        conn.close()
    print(f"Coverage: {passing}/{len(workload)} PASS, {errors} SQL errors, {mismatches} mismatches")
    return 1 if mismatches or errors else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rusthouse-bin", required=True, help="Path to release RustHouse CLI")
    parser.add_argument("--rusthouse-db", required=True, help="RustHouse database directory")
    parser.add_argument("--duckdb-db", required=True, help="DuckDB reference database")
    parser.add_argument("--repeats", type=int, default=3, help="Measured CLI runs (default: 3)")
    args = parser.parse_args()
    if args.repeats < 1 or args.repeats > 100:
        parser.error("--repeats must be between 1 and 100")
    try:
        return run_suite(args.rusthouse_bin, args.rusthouse_db, args.duckdb_db, args.repeats)
    except (OSError, RuntimeError, ValueError) as error:
        print(f"parity runner: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Load the synthetic ClickBench fixture and verify source/reference row counts."""
import argparse
import math
import sqlite3
from contextlib import closing
import sys
import time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql, emit_json
from fixture import FIELDS


def load_fixture(binary, database, fixture_dir, timeout=120, execute=None):
    if (type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0):
        raise ValueError("timeout must be finite and positive")
    root = Path(fixture_dir)
    tsv = root / "hits.tsv"
    with tsv.open(encoding="utf-8") as source:
        actual_header = source.readline().rstrip("\r\n").split("\t")
        if actual_header != [name for name, _, _ in FIELDS]:
            raise ValueError("hits TSV column names differ from the fixture schema")
        expected = 0
        for record in source:
            if len(record.rstrip("\r\n").split("\t")) != len(FIELDS):
                raise ValueError(f"hits TSV row {expected + 1} has an invalid column count")
            expected += 1
    reference_path = root / "hits.sqlite"
    if not reference_path.is_file():
        raise ValueError("SQLite fixture reference is missing")
    with closing(sqlite3.connect(reference_path)) as reference:
        sqlite_rows = reference.execute("SELECT COUNT(*) FROM hits").fetchone()[0]
        if expected < 1 or sqlite_rows != expected:
            raise ValueError(f"source row count {expected} differs from SQLite {sqlite_rows}")
        columns = ", ".join(name for name, _, _ in FIELDS)
        with tsv.open(encoding="utf-8") as source:
            source.readline()
            rows = reference.execute(f"SELECT {columns} FROM hits ORDER BY rowid")
            for number, (record, values) in enumerate(zip(source, rows, strict=True), 1):
                actual = record.rstrip("\r\n").split("\t")
                if actual != [str(value) for value in values]:
                    raise ValueError(f"hits TSV row {number} differs from SQLite reference")
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    create = (root / "create.sql").read_text().strip().rstrip(";")
    copy = (root / "copy.sql").read_text().strip().rstrip(";")
    if len(copy.encode("utf-8")) > 4096:
        raise ValueError("COPY SQL exceeds statement size limit")
    execute(create)
    started = time.perf_counter()
    execute(copy)
    elapsed = time.perf_counter() - started
    result = execute("SELECT COUNT(*) FROM hits").strip()
    if result != str(expected).encode("ascii"):
        raise ValueError(f"RustHouse row count {result!r} differs from expected {expected}")
    return {"workload": "clickbench-synthetic-import", "official_clickbench": False,
            "rows": expected, "copy_seconds": elapsed,
            "copy_rows_per_second": expected / elapsed if elapsed else None}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--fixture-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    try:
        report = load_fixture(args.binary, args.database, args.fixture_dir, args.timeout)
    except (OSError, RuntimeError, ValueError, sqlite3.Error) as error:
        parser.exit(1, f"ClickBench import failed: {error}\n")
    emit_json(report, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

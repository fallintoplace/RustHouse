#!/usr/bin/env python3
"""Load converted TPC-H TSV into a fresh RustHouse local database.

This measures loading separately from SQL query latency. The import is
non-transactional across tables, so a failed import must use a fresh database.
"""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import cli_sql
from schema import TABLES, create_sql, names

MAX_RECORD_BYTES = 8 * 1024 * 1024


def inspect_tsv(path, table):
    with Path(path).open("rb") as stream:
        header = stream.readline(MAX_RECORD_BYTES + 1)
        if len(header) > MAX_RECORD_BYTES:
            raise ValueError(f"{table}: oversized TSV header")
        header = header.rstrip(b"\n").rstrip(b"\r")
        if header != "\t".join(names(table)).encode("utf-8"):
            raise ValueError(f"{table}: header does not match expected dbgen columns")
        count = 0
        expected_width = len(names(table))
        for record in iter(lambda: stream.readline(MAX_RECORD_BYTES + 1), b""):
            if len(record) > MAX_RECORD_BYTES or not record.endswith(b"\n") or len(
                record.removesuffix(b"\n").removesuffix(b"\r").split(b"\t")
            ) != expected_width:
                raise ValueError(f"{table}: malformed TSV data row {count + 1}")
            count += 1
    if count < 1:
        raise ValueError(f"{table}: dataset is empty")
    return count


def copy_statement(table, source):
    escaped = source.as_posix().replace("'", "''")
    query = f"COPY {table} FROM '{escaped}' WITH (FORMAT TSV, HEADER TRUE)"
    if len(query.encode("utf-8")) > 4096:
        raise ValueError("COPY statement exceeds RustHouse SQL limit")
    return query


def load_tables(binary, database, input_dir, selected=None, timeout=3600, execute=None):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected):
        raise ValueError("tables must be a nonempty unique selection")
    if any(table not in TABLES for table in selected):
        raise ValueError("unknown table in import selection")
    if timeout <= 0:
        raise ValueError("timeout must be positive")
    sources = {}
    for table in selected:
        source = (Path(input_dir) / (table + ".tsv")).resolve()
        sources[table] = (source, inspect_tsv(source, table))
    if execute is None:
        execute = lambda sql: cli_sql(binary, database, sql, timeout)
    reports = []
    for table in selected:
        source, expected = sources[table]
        execute(create_sql(table))
        query = copy_statement(table, source)
        started = time.perf_counter()
        execute(query)
        elapsed = time.perf_counter() - started
        actual = execute(f"SELECT count(*) FROM {table}").strip()
        if actual != str(expected).encode("ascii"):
            raise ValueError(f"{table}: imported {actual!r}, expected {expected}")
        reports.append({
            "table": table, "rows": expected,
            "copy_seconds": elapsed,
            "copy_rows_per_second": expected / elapsed if elapsed > 0 else None,
        })
    return {"engine": "rusthouse-cli", "official_tpch_score": False,
            "timing_scope": "per-table COPY statement only", "tables": reports}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    parser.add_argument("--timeout", type=float, default=3600)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = load_tables(args.binary, args.database, args.input_dir, args.table, args.timeout)
    except (OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"TPC-H import failed: {error}\n")
    result = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(result, encoding="utf-8")
    print(result, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

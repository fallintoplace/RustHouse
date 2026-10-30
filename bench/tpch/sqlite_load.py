#!/usr/bin/env python3
"""Build an independent SQLite reference database from converted dbgen TSV."""
import argparse
import csv
import json
import os
import sqlite3
import tempfile
import time
from datetime import date
from pathlib import Path

from schema import TABLES, names

SQLITE_INT_MAX = 2**63 - 1
SQLITE_INT_MIN = -(2**63)
MAX_RECORD_BYTES = 8 * 1024 * 1024


def convert_cell(raw, kind):
    if kind == "String":
        # The RustHouse dbgen converter doubles literal backslashes for TSV.
        return raw.replace("\\\\", "\\")
    if kind == "Date":
        try:
            if date.fromisoformat(raw).isoformat() == raw:
                return raw
        except ValueError:
            pass
        raise ValueError(f"invalid ISO date: {raw!r}")
    try:
        number = int(raw)
    except ValueError as error:
        raise ValueError(f"invalid integer: {raw!r}") from error
    if raw != str(number):
        raise ValueError("non-canonical integer in converted TSV")
    if not SQLITE_INT_MIN <= number <= SQLITE_INT_MAX:
        raise ValueError("integer exceeds SQLite's signed 64-bit range")
    if kind == "UInt64" and number < 0:
        raise ValueError("negative UInt64")
    return number


def load_table(connection, input_dir, table, batch_size=4096):
    if not 1 <= batch_size <= 65_536:
        raise ValueError("batch size must be in 1..65536")
    fields = TABLES[table]
    columns = ", ".join(
        f'"{name}" {"TEXT" if kind in ("String", "Date") else "INTEGER"}'
        for name, kind, _ in fields
    )
    connection.execute(f'CREATE TABLE "{table}" ({columns})')
    placeholders = ", ".join("?" for _ in fields)
    insert = f'INSERT INTO "{table}" VALUES ({placeholders})'
    count = 0
    batch = []
    path = Path(input_dir) / (table + ".tsv")
    started = time.perf_counter()
    with path.open("rb") as source:
        header_raw = source.readline(MAX_RECORD_BYTES + 1)
        if len(header_raw) > MAX_RECORD_BYTES:
            raise ValueError(f"{table}: oversized TSV header")
        header = header_raw.decode("utf-8").rstrip("\r\n").split("\t")
        if header != names(table):
            raise ValueError(f"{table}: unexpected TSV header")
        for line_number, raw in enumerate(
            iter(lambda: source.readline(MAX_RECORD_BYTES + 1), b""), 2
        ):
            if len(raw) > MAX_RECORD_BYTES or not raw.endswith(b"\n"):
                raise ValueError(f"{table}: oversized or truncated TSV row {line_number}")
            values = raw.decode("utf-8").rstrip("\r\n").split("\t")
            if len(values) != len(fields):
                raise ValueError(f"{table}: wrong column count on line {line_number}")
            try:
                converted = tuple(
                    convert_cell(raw, kind)
                    for raw, (_, kind, _) in zip(values, fields)
                )
            except ValueError as error:
                raise ValueError(f"{table}: line {line_number}: {error}") from error
            batch.append(converted)
            if len(batch) >= batch_size:
                connection.executemany(insert, batch)
                count += len(batch)
                batch.clear()
    if batch:
        connection.executemany(insert, batch)
        count += len(batch)
    if count == 0:
        raise ValueError(f"{table}: no input rows")
    actual = connection.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
    if actual != count:
        raise ValueError(f"{table}: row count mismatch")
    return {"table": table, "rows": count, "seconds": time.perf_counter() - started}


def load_database(input_dir, database, selected=None, batch_size=4096):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected):
        raise ValueError("table selection must be nonempty and unique")
    if any(name not in TABLES for name in selected):
        raise ValueError("unknown dbgen table")
    database = Path(database)
    if database.exists():
        raise ValueError("refusing to overwrite existing reference database")
    database.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix="tpch-sqlite-", suffix=".db",
                                    dir=database.parent)
    os.close(fd)
    temp = Path(temp_name)
    try:
        connection = sqlite3.connect(temp)
        try:
            connection.execute("BEGIN")
            reports = [load_table(connection, input_dir, table, batch_size)
                       for table in selected]
            connection.commit()
        finally:
            connection.close()
        if database.exists():
            raise ValueError("target database was created while loading")
        os.link(temp, database)
    finally:
        temp.unlink(missing_ok=True)
    return {
        "engine": "sqlite",
        "input_format": "rusthouse-converted-fixed-point-tsv",
        "official_tpch_score": False,
        "tables": reports,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    parser.add_argument("--batch-size", type=int, default=4096)
    args = parser.parse_args(argv)
    try:
        report = load_database(args.input_dir, args.database, args.table,
                               args.batch_size)
    except (OSError, ValueError, sqlite3.DatabaseError) as error:
        parser.exit(1, f"SQLite import failed: {error}\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

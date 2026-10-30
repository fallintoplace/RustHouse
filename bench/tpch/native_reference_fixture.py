#!/usr/bin/env python3
"""Load dbgen .tbl into DuckDB with native decimals for full Q1-Q22 references.

RustHouse's fixed-point TSV schema is intentionally NOT used for this database.
Do not compare its scaled numeric outputs to this native decimal reference
without proving equivalent logical values in the RustHouse query engine.
"""
import argparse
import csv
import json
import os
import subprocess
import tempfile
from pathlib import Path

from convert_dbgen import MAX_LINE, parse_scaled_number
from schema import TABLES, names


def duckdb_type(kind, scale):
    if scale == 100:
        return "DECIMAL(18,2)"
    if scale == 10000:
        return "DECIMAL(18,4)"
    return {
        "UInt64": "UBIGINT",
        "Int64": "BIGINT",
        "String": "VARCHAR",
        "Date": "DATE",
    }[kind]


def create_statement(table):
    if table not in TABLES:
        raise ValueError("unknown TPC-H table")
    return f"CREATE TABLE {table} (" + ", ".join(
        f"{column} {duckdb_type(kind, scale)}"
        for column, kind, scale in TABLES[table]
    ) + ")"


def convert_native_table(source, target, table):
    """Preserve dbgen decimal values rather than scaling them to integers."""
    if table not in TABLES:
        raise ValueError("unknown TPC-H table")
    source, target = Path(source), Path(target)
    if source.resolve() == target.resolve():
        raise ValueError("source and destination must differ")
    count = 0
    with source.open("rb") as reader, target.open("w", newline="", encoding="utf-8") as dest:
        writer = csv.writer(dest, delimiter="\t", lineterminator="\n")
        writer.writerow(names(table))
        for number, raw in enumerate(reader, 1):
            if len(raw) > MAX_LINE:
                raise ValueError(f"{table} row {number}: exceeds 8 MiB")
            try:
                cells = raw.decode("utf-8").rstrip("\r\n").split("|")
            except UnicodeDecodeError as error:
                raise ValueError(f"{table} row {number}: invalid UTF-8") from error
            if len(cells) != len(TABLES[table]) + 1 or cells[-1] != "":
                raise ValueError(f"{table} row {number}: invalid dbgen field count")
            for value, (_, kind, scale) in zip(cells[:-1], TABLES[table]):
                if kind == "String":
                    if not value or any(ord(char) < 32 or ord(char) == 127 for char in value):
                        raise ValueError(f"{table} row {number}: invalid string")
                elif kind == "Date":
                    from datetime import date
                    if len(value) != 10 or date.fromisoformat(value).isoformat() != value:
                        raise ValueError(f"{table} row {number}: invalid date")
                else:
                    parse_scaled_number(value, scale)
            writer.writerow(cells[:-1])
            count += 1
    if count == 0:
        raise ValueError(f"{table}: empty dbgen input")
    return count


def escape_literal(value):
    return str(value).replace("'", "''")


def copy_statement(table, path):
    if table not in TABLES:
        raise ValueError("unknown TPC-H table")
    return (
        f"COPY {table} FROM '{escape_literal(path)}' "
        "(FORMAT CSV, DELIMITER '\\t', HEADER TRUE)"
    )


def load(duckdb, database, source_dir, selected=None, timeout=3600, execute=None):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected) or any(
        name not in TABLES for name in selected
    ):
        raise ValueError("tables must be a nonempty unique selection")
    database = Path(database)
    if database.exists():
        raise ValueError("refusing to overwrite an existing DuckDB database")
    database.parent.mkdir(parents=True, exist_ok=True)
    if execute is None:
        execute = lambda cmd: subprocess.run(
            cmd, capture_output=True, check=False, timeout=timeout
        )
    results = []
    with tempfile.TemporaryDirectory(prefix="tpch-native-decimals-", dir=database.parent) as work:
        work = Path(work)
        staging_db = work / "reference.duckdb"
        native_tsv = work / "source.tsv"
        for table in selected:
            rows = convert_native_table(Path(source_dir) / f"{table}.tbl",
                                        native_tsv, table)
            sql = create_statement(table) + "; " + copy_statement(table, native_tsv) + ";"
            proc = execute([str(duckdb), "-no-init", "-batch",
                            str(staging_db), "-c", sql])
            if proc.returncode:
                stderr = proc.stderr.decode("utf-8", "replace")[-500:]
                raise RuntimeError(f"{table}: DuckDB import failed: {stderr}")
            check = execute([str(duckdb), "-no-init", "-batch", "-list", "-noheader",
                             str(staging_db), "-c", f"SELECT count(*) FROM {table}"])
            if check.returncode or check.stdout.strip() != str(rows).encode("ascii"):
                raise ValueError(f"{table}: DuckDB imported a different number of rows")
            results.append({"table": table, "rows": rows})
        # A failed table import never publishes a partial reference database.
        os.replace(staging_db, database)
    return {
        "engine": "duckdb", "native_decimal_types": True,
        "official_tpch_score": False, "tables": results,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duckdb", default="duckdb")
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--dbgen-dir", required=True, type=Path)
    parser.add_argument("--table", action="append", choices=list(TABLES))
    parser.add_argument("--timeout", type=float, default=3600)
    args = parser.parse_args(argv)
    try:
        result = load(args.duckdb, args.database, args.dbgen_dir,
                      args.table, args.timeout)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"native TPC-H reference import failed: {error}\n")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

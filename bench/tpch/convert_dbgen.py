#!/usr/bin/env python3
"""Stream dbgen .tbl files to RustHouse TSV using explicit fixed-point scales."""
import argparse
import datetime
import re
from decimal import Decimal, InvalidOperation
from pathlib import Path

from schema import TABLES, names

MAX_LINE = 8 * 1024 * 1024
DECIMAL = re.compile(r"-?[0-9]+(?:\.[0-9]+)?\Z")
INTEGER = re.compile(r"-?[0-9]+\Z")


def convert_value(raw, kind, scale):
    if kind == "String":
        if not raw or raw == r"\N" or any(c in raw for c in ("\t", "\r", "\n")):
            raise ValueError("empty, NULL-marker, or control-character string")
        return raw.replace("\\", "\\\\")
    if kind == "Date":
        if len(raw) != 10 or datetime.date.fromisoformat(raw).isoformat() != raw:
            raise ValueError("date must be YYYY-MM-DD")
        return raw
    if scale != 1:
        if not DECIMAL.fullmatch(raw):
            raise ValueError("invalid fixed-point decimal")
        try:
            scaled = Decimal(raw) * scale
        except InvalidOperation as error:
            raise ValueError("invalid decimal") from error
        if scaled != scaled.to_integral_value():
            raise ValueError("decimal precision exceeds supported fixed-point scale")
        value = int(scaled)
    else:
        if not INTEGER.fullmatch(raw):
            raise ValueError("integer expected")
        value = int(raw)
    low, high = (0, 2**64 - 1) if kind == "UInt64" else (-2**63, 2**63 - 1)
    if not low <= value <= high:
        raise ValueError("integer outside RustHouse column range")
    return str(value)


def convert_table(source, target, table):
    if table not in TABLES:
        raise ValueError(f"unknown dbgen table: {table}")
    source, target = Path(source), Path(target)
    if source.resolve() == target.resolve():
        raise ValueError("source and target must differ")
    target.parent.mkdir(parents=True, exist_ok=True)
    temp = target.with_name(target.name + ".tmp")
    rows = 0
    try:
        with source.open("r", encoding="utf-8", newline="") as reader, \
                temp.open("w", encoding="utf-8", newline="\n") as writer:
            writer.write("\t".join(names(table)) + "\n")
            for number, line in enumerate(reader, 1):
                if len(line.encode("utf-8")) > MAX_LINE:
                    raise ValueError(f"{table} row {number}: exceeds 8 MiB")
                fields = line.rstrip("\n").rstrip("\r").split("|")
                if len(fields) != len(TABLES[table]) + 1 or fields[-1] != "":
                    raise ValueError(f"{table} row {number}: wrong field count or missing trailing pipe")
                try:
                    converted = [
                        convert_value(raw, kind, scale)
                        for raw, (_, kind, scale) in zip(fields[:-1], TABLES[table])
                    ]
                except ValueError as error:
                    raise ValueError(f"{table} row {number}: {error}") from error
                writer.write("\t".join(converted) + "\n")
                rows += 1
        temp.replace(target)
    finally:
        temp.unlink(missing_ok=True)
    return rows


def convert_directory(source_dir, target_dir, selected=None):
    selected = tuple(selected or TABLES)
    if not selected or any(table not in TABLES for table in selected):
        raise ValueError("select one or more known TPC-H tables")
    counts = {}
    for table in selected:
        src = Path(source_dir) / (table + ".tbl")
        dst = Path(target_dir) / (table + ".tsv")
        counts[table] = convert_table(src, dst, table)
    return counts


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    args = parser.parse_args(argv)
    try:
        counts = convert_directory(args.input_dir, args.output_dir, args.table)
    except (OSError, ValueError) as error:
        parser.exit(1, f"conversion failed: {error}\n")
    for table, rows in counts.items():
        print(f"{table}: {rows} converted rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Stream dbgen .tbl files to RustHouse TSV using explicit fixed-point scales."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import datetime
import re
from decimal import Decimal, InvalidOperation
from pathlib import Path

from schema import TABLES, names

MAX_LINE = 8 * 1024 * 1024
DECIMAL = re.compile(r"-?[0-9]+(?:\.[0-9]+)?\Z")
INTEGER = re.compile(r"-?[0-9]+\Z")


def parse_scaled_number(raw, scale):
    if scale != 1:
        if not DECIMAL.fullmatch(raw):
            raise ValueError("invalid fixed-point decimal")
        # dbgen uses decimal scales of 100 and 10000. Keep their exact
        # conversion in integer arithmetic instead of building millions of
        # Decimal objects during SF1 and larger imports.
        digits = str(scale)
        if scale > 0 and digits.startswith("1") and set(digits[1:]) <= {"0"}:
            precision = len(digits) - 1
            negative = raw.startswith("-")
            magnitude = raw[1:] if negative else raw
            whole, _, fraction = magnitude.partition(".")
            if len(fraction) > precision:
                if any(char != "0" for char in fraction[precision:]):
                    raise ValueError("decimal precision exceeds supported fixed-point scale")
                fraction = fraction[:precision]
            value = int(whole) * scale + int(fraction.ljust(precision, "0") or "0")
            return -value if negative else value
        try:
            scaled = Decimal(raw) * scale
        except InvalidOperation as error:
            raise ValueError("invalid decimal") from error
        if scaled != scaled.to_integral_value():
            raise ValueError("decimal precision exceeds supported fixed-point scale")
        return int(scaled)
    if not INTEGER.fullmatch(raw):
        raise ValueError("integer expected")
    return int(raw)


def convert_value(raw, kind, scale):
    if kind == "String":
        if not raw or raw == r"\N" or any(ord(char) < 32 or ord(char) == 127 for char in raw):
            raise ValueError("empty, NULL-marker, or control-character string")
        return raw.replace("\\", "\\\\")
    if kind == "Date":
        if len(raw) != 10 or datetime.date.fromisoformat(raw).isoformat() != raw:
            raise ValueError("date must be YYYY-MM-DD")
        return raw
    value = parse_scaled_number(raw, scale)
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
        with source.open("rb") as reader, \
                temp.open("w", encoding="utf-8", newline="\n") as writer:
            writer.write("\t".join(names(table)) + "\n")
            for number, raw_line in enumerate(iter(lambda: reader.readline(MAX_LINE + 1), b""), 1):
                if len(raw_line) > MAX_LINE:
                    raise ValueError(f"{table} row {number}: exceeds 8 MiB")
                try:
                    line = raw_line.decode("utf-8")
                except UnicodeDecodeError as error:
                    raise ValueError(f"{table} row {number}: invalid UTF-8") from error
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


def convert_directory(source_dir, target_dir, selected=None, jobs=1):
    selected = tuple(TABLES if selected is None else selected)
    if not selected or len(set(selected)) != len(selected) or any(
        table not in TABLES for table in selected
    ):
        raise ValueError("select a nonempty unique set of known TPC-H tables")
    if type(jobs) is not int or not 1 <= jobs <= len(TABLES):
        raise ValueError("jobs must be an integer between 1 and 8")

    def convert_one(table):
        src = Path(source_dir) / (table + ".tbl")
        dst = Path(target_dir) / (table + ".tsv")
        return table, convert_table(src, dst, table)

    # Table files are independent and can be converted on separate workers.
    # Maintain deterministic table-order reporting regardless of completion order.
    if jobs == 1:
        return dict(map(convert_one, selected))
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        return dict(pool.map(convert_one, selected))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--table", action="append", choices=tuple(TABLES))
    parser.add_argument("--jobs", type=int, default=1,
                        help="Parallel dbgen table converters (1 to 8)")
    args = parser.parse_args(argv)
    try:
        counts = convert_directory(args.input_dir, args.output_dir, args.table,
                                   jobs=args.jobs)
    except (OSError, ValueError) as error:
        parser.exit(1, f"conversion failed: {error}\n")
    for table, rows in counts.items():
        print(f"{table}: {rows} converted rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
